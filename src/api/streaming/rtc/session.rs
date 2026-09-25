use crate::api::streaming::rtc::clock::RtpClockProbe;
use crate::api::streaming::rtc::media::{AudioReceiver, VideoReceiver};
use crate::api::streaming::rtc::transport::RtcTransport;
use crate::streaming::input::{GamepadFrame, PointerEvent};
use crate::streaming::video::metrics::METRICS;
use crate::streaming::video::{DecoderConfig, DirectVideoOutput};
use anyhow::{Context, Result};
use crate::api::streaming::rtc::peer::RTCPeerConnection;
use rtc::peer_connection::event::{RTCDataChannelEvent, RTCPeerConnectionEvent, RTCTrackEvent};
use rtc::peer_connection::message::RTCMessage;
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::peer_connection::state::RTCPeerConnectionState;
use rtc::peer_connection::transport::RTCIceCandidateInit;
use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
use rtc::sansio::Protocol;
use rtc::statistics::report::RTCStatsReportEntry;
use rtc::statistics::StatsSelector;
use rtcp::sender_report::SenderReport;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

const KEYFRAME_REQUEST_COOLDOWN: Duration = Duration::from_millis(300);
const INITIAL_VIDEO_GRACE: Duration = Duration::from_millis(500);
const INITIAL_VIDEO_KEYFRAME_INTERVAL: Duration = Duration::from_millis(500);

pub(crate) struct RtcSessionConfig {
    pub stun_server: &'static str,
    pub route_probe: &'static str,
    pub audio_sample_rate: u32,
    pub audio_payload_type: u8,
    pub requested_video_size: (u32, u32),
    pub decoder: DecoderConfig,
}

/// Provider-specific hooks invoked by the reusable RTC session.
pub(crate) trait RtcSessionBackend {
    fn pump_microphone(&mut self, _peer: &mut RTCPeerConnection, _connected: bool) {}
    fn begin_chat_negotiation(&mut self, _peer: &mut RTCPeerConnection) -> Option<RTCSessionDescription> { None }
    fn finish_chat_negotiation(&mut self, _peer: &mut RTCPeerConnection, _answer: Result<String>) -> bool { false }
    fn microphone_status(&self) -> String { String::new() }
    fn handle_channel_open(
        &mut self,
        peer: &mut RTCPeerConnection,
        channel_id: rtc::data_channel::RTCDataChannelId,
    );
    fn handle_channel_message(
        &mut self,
        peer: &mut RTCPeerConnection,
        channel_id: rtc::data_channel::RTCDataChannelId,
        data: &[u8],
    );
    fn send_gamepad_frame(&mut self, peer: &mut RTCPeerConnection, frame: GamepadFrame) -> bool;
    fn send_pointer_event(&mut self, peer: &mut RTCPeerConnection, event: PointerEvent);
    fn notify_keyframe_requested(&mut self, peer: &mut RTCPeerConnection);
    fn server_video_size(&self) -> Option<(u32, u32)>;
    fn send_rendered_frame(&mut self, peer: &mut RTCPeerConnection, frame: crate::streaming::video::timing::PresentedFrame) -> bool;
}

pub(crate) struct RtcSession<B: RtcSessionBackend> {
    pub(crate) peer: RTCPeerConnection,
    pub(crate) transport: RtcTransport,
    pub(crate) backend: B,
    pub connection_state: RTCPeerConnectionState,
    pub(crate) video: VideoReceiver,
    pub(crate) audio: AudioReceiver,
    last_keyframe_request: Option<Instant>,
    initial_video_watchdog_started_at: Option<Instant>,
    last_initial_video_keyframe_request: Option<Instant>,
    pub status: String,
    requested_video_size: (u32, u32),
    video_clock: RtpClockProbe,
    audio_clock: RtpClockProbe,
    video_rate: super::feedback::ReceiveRate,
    video_ceiling: super::feedback::VideoCeiling,
    catch_up: crate::streaming::video::catch_up::CatchUp,
    direct_output: Arc<DirectVideoOutput>,
}

impl<B: RtcSessionBackend> RtcSession<B> {
    pub async fn new(
        mut peer: RTCPeerConnection,
        backend: B,
        config: RtcSessionConfig,
        direct_output: Arc<DirectVideoOutput>,
    ) -> Result<Self> {
        let transport =
            RtcTransport::bind(&mut peer, config.stun_server, config.route_probe).await?;
        let video = VideoReceiver::new(config.decoder, Arc::clone(&direct_output))?;
        crate::streaming::video::trace::reset();

        Ok(Self {
            peer,
            transport,
            backend,
            connection_state: RTCPeerConnectionState::New,
            video,
            audio: AudioReceiver::new(config.audio_sample_rate, config.audio_payload_type),
            last_keyframe_request: None,
            initial_video_watchdog_started_at: None,
            last_initial_video_keyframe_request: None,
            status: "Negotiating WebRTC connection".to_owned(),
            requested_video_size: config.requested_video_size,
            video_clock: RtpClockProbe::new(90_000),
            audio_clock: RtpClockProbe::new(i64::from(config.audio_sample_rate)),
            video_rate: super::feedback::ReceiveRate::new(),
            video_ceiling: Default::default(),
            catch_up: Default::default(),
            direct_output,
        })
    }

    pub fn create_offer(&mut self) -> Result<RTCSessionDescription> {
        let offer = self
            .peer
            .create_offer_with_video_bandwidth(None, super::bandwidth::VIDEO_CEILING_BPS)
            .context("failed to create rtc offer")?;
        self.peer
            .set_local_description(offer.clone())
            .context("failed to set local rtc description")?;
        Ok(offer)
    }

    pub fn set_remote_answer(&mut self, sdp: impl Into<String>) -> Result<()> {
        let sdp = sdp.into();
        let answer = RTCSessionDescription::answer(sdp.clone());
        self.peer
            .set_remote_description(answer?)
            .context("failed to set remote rtc answer")?;
        self.refresh_negotiated_feedback();
        Ok(())
    }

    pub(crate) fn refresh_negotiated_feedback(&mut self) {
        if let Some(answer) = self.peer.current_remote_description() {
            self.video_ceiling.answer(&answer.sdp);
            self.video.set_nack_payloads(super::feedback::feedback_payloads(&answer.sdp, "nack"));
        }
    }

    pub fn close(&mut self) -> Result<()> {
        self.peer
            .close()
            .context("failed to close rtc peer connection")
    }

    pub fn add_remote_candidate(&mut self, candidate: RTCIceCandidateInit) -> Result<()> {
        self.peer
            .add_remote_candidate(candidate)
            .context("failed to add remote ICE candidate")
    }

    pub(crate) fn refresh_video(&mut self) {
        self.video.refresh();
        let now = Instant::now();
        self.catch_up.requested(now);
        self.request_keyframe(true, now);
    }

    pub async fn pump(&mut self) -> Result<Vec<RTCIceCandidateInit>> {
        self.transport.flush(&mut self.peer).await;
        self.transport.receive(&mut self.peer);
        let gathered_candidates = self.handle_peer_events();
        let mut keyframe_requested = self.handle_peer_messages();
        self.backend.pump_microphone(&mut self.peer, self.connection_state == RTCPeerConnectionState::Connected);
        self.video.drain_decoder(&mut keyframe_requested);
        self.video.request_missing_packets(&mut self.peer);

        // Feedback identifies a matched output that has completed rendering.
        // Decode-only, replaced and unknown-PTS pictures never enter this slot.
        let presented = self.direct_output.presentation.lock().ok().and_then(|mut state| state.take());
        if let Some(frame) = presented {
            let sent = self.backend.send_rendered_frame(&mut self.peer, frame);
            let counter = if sent { &METRICS.frame_feedback_sent } else { &METRICS.frame_feedback_failed };
            counter.fetch_add(1, Ordering::Relaxed);
            crate::streaming::video::trace::record("frame_feedback", frame.timing.rtp_timestamp, u64::from(sent));
        }

        // Materialize admitted ephemeral reports while the sampled send capacity
        // still applies. A timeout can shrink SCTP's congestion window.
        self.transport.flush(&mut self.peer).await;
        let now = Instant::now();
        self.video_ceiling.update_arrival_feedback(
            super::reports::video_arrival_packets(), self.transport.twcc_sent(), now);
        if let Some(sample) = self.video_clock.timing()
            && self.catch_up.observe(sample.timestamp, sample.received_at,
                sample.added_delay_ms, self.video.decoder.queued_frames(),
                self.video.recovering(), now)
        {
            crate::streaming::video::trace::record("lag_keyframe_request",
                sample.timestamp, sample.added_delay_ms);
            self.video.refresh();
            keyframe_requested = true;
        }
        // rtc-rs is sans-I/O, so its expired internal timer must be advanced by our pump.
        if let Some(deadline) = self.peer.poll_timeout()
            && now >= deadline
        {
            let _ = self.peer.handle_timeout(now);
        }
        if self.initial_video_keyframe_due(now) {
            eprintln!("No initial video RTP after WebRTC connected; requesting a keyframe");
            keyframe_requested = true;
        }
        self.request_keyframe(keyframe_requested, now);
        if self.video_ceiling.due(now)
            && let Some(success) = self.video.request_bitrate_ceiling(
                &mut self.peer, self.video_ceiling.target_bps())
        {
            self.video_ceiling.attempted(now, success);
            crate::streaming::video::trace::record("remb_queued", 0, u64::from(success));
        }
        // Send acknowledgements, reports and recovery requests generated by this receive
        // pass before waiting for the next packet/timer. Security processing is unchanged.
        self.transport.flush(&mut self.peer).await;
        if let Some(status) = self.video.status(now) {
            let link = self.connection_debug(now);
            let receive = self.transport.take_receive_summary();
            let rate = self.video_rate.summary(now);
            let ceiling = self.video_ceiling.summary(self.video_rate.latest_kbps);
            let feedback = format!("Video payload:{rate}\n{ceiling}\n{}\n{}\nCatch-up requests:{}\nSDP video ceiling:{}k", super::reports::summary(), self.video.repair_summary(), self.catch_up.requests(), super::bandwidth::VIDEO_CEILING_BPS / 1000);
            let (requested_width, requested_height) = self.requested_video_size;
            let server_size = self
                .backend
                .server_video_size()
                .map(|(width, height)| format!("{width}x{height}"))
                .unwrap_or_else(|| "?".to_owned());
            let microphone = self.backend.microphone_status();
            self.status = format!(
                "Build: RX Test {} revision {}\nXbox requested:{requested_width}x{requested_height} server:{server_size}\n{status}\n{link}\n{receive}\n{feedback}\n{microphone}",
                crate::build_info::NUMBER, crate::build_info::REVISION,
            );
            crate::streaming::video::trace::status_snapshot(&self.status);
            eprintln!("{}", self.status);
        }

        Ok(gathered_candidates)
    }

    pub(crate) fn video_timing(&self) -> Option<crate::streaming::video::freshness::VideoTiming> {
        let sample = self.video_clock.timing()?;
        let rendered = self.direct_output.presentation.lock().ok().and_then(|state| state.latest);
        Some(sample.with_presented_frame(rendered, Instant::now()))
    }

    fn connection_debug(&mut self, now: Instant) -> String {
        // This report is queried once per second, when the video status is updated.
        let stats = self.peer.get_stats(now, StatsSelector::None);
        let pair = stats
            .transport()
            .and_then(|transport| stats.get(&transport.selected_candidate_pair_id))
            .and_then(|entry| match entry {
                RTCStatsReportEntry::IceCandidatePair(pair) => Some(pair),
                _ => None,
            });
        let ice = if let Some(pair) = pair {
            let remote = stats
                .get(&pair.remote_candidate_id)
                .and_then(|entry| match entry {
                    RTCStatsReportEntry::RemoteCandidate(candidate) => {
                        Some(format!("{:?}", candidate.candidate_type))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| "?".to_owned());
            format!("{}ms/{remote}", (pair.current_round_trip_time * 1_000.0) as u32)
        } else {
            "?".to_owned()
        };
        let av_age = match (self.video_clock.age_ms(), self.audio_clock.age_ms()) {
            (Some(video), Some(audio)) => format!("{}ms", video - audio),
            _ => "?".to_owned(),
        };
        format!(
            "Link ICE:{ice} DCbuf:{}/{} | SR offset* V:{} A:{} V-A:{av_age}",
            METRICS.input_buffered_high.load(Ordering::Relaxed),
            METRICS.input_buffered_events.load(Ordering::Relaxed),
            self.video_clock.summary(now),
            self.audio_clock.summary(now),
        )
    }

    fn initial_video_keyframe_due(&mut self, now: Instant) -> bool {
        if self.video.received_packet {
            self.initial_video_watchdog_started_at = None;
            self.last_initial_video_keyframe_request = None;
            return false;
        }
        if self.connection_state != RTCPeerConnectionState::Connected {
            self.initial_video_watchdog_started_at = None;
            self.last_initial_video_keyframe_request = None;
            return false;
        }

        let connected_at = *self.initial_video_watchdog_started_at.get_or_insert(now);
        if now.duration_since(connected_at) < INITIAL_VIDEO_GRACE {
            return false;
        }
        if self
            .last_initial_video_keyframe_request
            .is_some_and(|requested_at| {
                now.duration_since(requested_at) < INITIAL_VIDEO_KEYFRAME_INTERVAL
            })
        {
            return false;
        }

        self.last_initial_video_keyframe_request = Some(now);
        true
    }

    fn handle_peer_events(&mut self) -> Vec<RTCIceCandidateInit> {
        let mut gathered_candidates = Vec::new();
        while let Some(event) = self.peer.poll_event() {
            match event {
                RTCPeerConnectionEvent::OnConnectionStateChangeEvent(state) => {
                    self.connection_state = state;
                    self.status = format!("WebRTC connection: {state:?}");
                }
                RTCPeerConnectionEvent::OnIceCandidateEvent(ice_event) => {
                    if let Ok(candidate) = ice_event.candidate.to_json() {
                        gathered_candidates.push(candidate);
                    }
                }
                RTCPeerConnectionEvent::OnIceCandidateErrorEvent(error) => {
                    eprintln!(
                        "ICE candidate error from {} ({}:{}): {} {}",
                        error.url, error.address, error.port, error.error_code, error.error_text
                    );
                }
                RTCPeerConnectionEvent::OnTrack(RTCTrackEvent::OnOpen(init)) => {
                    let track_kind = self
                        .peer
                        .rtp_receiver(init.receiver_id)
                        .map(|receiver| receiver.track().kind());
                    match track_kind {
                        Some(RtpCodecKind::Video) => {
                            self.video.open(init.track_id, init.receiver_id, init.ssrc);
                            self.status = "Receiving video track".to_owned();
                        }
                        Some(RtpCodecKind::Audio) => {
                            self.audio.open(init.track_id);
                            self.status = "Receiving audio track".to_owned();
                        }
                        _ => {}
                    }
                }
                RTCPeerConnectionEvent::OnDataChannel(RTCDataChannelEvent::OnOpen(channel_id)) => {
                    self.backend.handle_channel_open(&mut self.peer, channel_id);
                }
                RTCPeerConnectionEvent::OnDataChannel(
                    RTCDataChannelEvent::OnBufferedAmountHigh(_),
                ) => {
                    METRICS.input_buffered_high.store(1, Ordering::Relaxed);
                    METRICS.input_buffered_events.fetch_add(1, Ordering::Relaxed);
                }
                RTCPeerConnectionEvent::OnDataChannel(
                    RTCDataChannelEvent::OnBufferedAmountLow(_),
                ) => {
                    METRICS.input_buffered_high.store(0, Ordering::Relaxed);
                }
                _ => {}
            }
        }
        gathered_candidates
    }

    fn handle_peer_messages(&mut self) -> bool {
        let mut keyframe_requested = false;
        for (clock_rate, report) in super::reports::take_clock_reports() {
            if clock_rate == 90_000 {
                self.video_clock.sender_report(&report);
            } else {
                self.audio_clock.sender_report(&report);
            }
        }
        while let Some(message) = self.peer.poll_read() {
            match message {
                RTCMessage::RtpPacket(track_id, packet) => {
                    if self.video.handles(&track_id) {
                        self.video_ceiling.observe_payload(packet.header.payload_type);
                        self.video_rate.receive(packet.payload.len(), Instant::now());
                        self.video_clock.receive(packet.header.timestamp);
                        if let Some(timing) = self.video_clock.timing() {
                            self.video_ceiling.receive(packet.payload.len(), timing.added_delay_ms, Instant::now());
                        }
                        self.video.receive(packet, &mut keyframe_requested);
                    } else if self.audio.handles(&track_id) {
                        self.audio_clock.receive(packet.header.timestamp);
                        self.audio.receive(packet);
                    }
                }
                RTCMessage::RtcpPacket(track_id, packets) => {
                    for packet in packets {
                        if let Some(sr) = packet.as_any().downcast_ref::<SenderReport>() {
                            if self.video.handles(&track_id) {
                                self.video_clock.sender_report(sr);
                            } else if self.audio.handles(&track_id) {
                                self.audio_clock.sender_report(sr);
                            }
                        }
                    }
                }
                RTCMessage::DataChannelMessage(channel_id, data_message) => {
                    self.backend.handle_channel_message(
                        &mut self.peer,
                        channel_id,
                        &data_message.data,
                    );
                }
                _ => {}
            }
        }
        keyframe_requested
    }

    fn request_keyframe(&mut self, requested: bool, now: Instant) {
        let cooldown_elapsed = self.last_keyframe_request.is_none_or(|requested_at| {
            now.duration_since(requested_at) >= KEYFRAME_REQUEST_COOLDOWN
        });
        if !requested || !cooldown_elapsed {
            return;
        }

        self.last_keyframe_request = Some(now);
        crate::streaming::video::trace::record("keyframe_request", 0, 0);
        self.backend.notify_keyframe_requested(&mut self.peer);
        self.video.request_keyframe(&mut self.peer);
    }
}

impl<B: RtcSessionBackend> Drop for RtcSession<B> {
    fn drop(&mut self) {
        // The remote Closed event and pump/socket errors can return from the
        // worker without calling close(). Save on ownership teardown so all
        // graceful exit paths persist diagnostics, including remote-first exit.
        // This cannot run if the OS forcibly terminates the application.
        self.video.decoder.shutdown();
        crate::streaming::video::trace::save(&self.status);
    }
}
