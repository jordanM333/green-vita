use crate::api::streaming::rtc::rtp;
use crate::streaming::video::{DecodedFrame, DecoderConfig, DirectVideoOutput, VideoDecodeWorker};
use anyhow::Result;
use bytes::Bytes;
use rtc::media_stream::MediaStreamTrackId;
use crate::api::streaming::rtc::peer::RTCPeerConnection;
use rtc::rtp::Packet;
use rtc::rtp_transceiver::RTCRtpReceiverId;
use std::sync::Arc;
use std::time::{Duration, Instant};

const STREAM_STATS_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Default)]
struct VideoStats {
    packets: u64,
    assembled: u64,
    submitted: u64,
    dropped: u64,
    missing_sequence: u64,
    incomplete_fua: u64,
    malformed: u64,
    queue_full: u64,
    sps_rejected: u64,
    idr_wait: u64,
    other: u64,
    sequence_jumps: u64,
    estimated_missing_packets: u64,
    late_packets: u64,
    duplicate_packets: u64,
    empty_packets: u64,
    idr: u64,
    post_damage_submitted: u64,
    decode_errors: u64,
    last_decode_error: Option<String>,
    last_sample_duration_us: Option<u64>,
    encoded_resolution: Option<(u32, u32)>,
}

pub(crate) struct VideoReceiver {
    track_id: Option<MediaStreamTrackId>,
    receiver_id: Option<RTCRtpReceiverId>,
    ssrc: Option<u32>,
    rtp: rtp::VideoRtp,
    order: super::reorder::PacketOrder<(Packet, Instant)>,
    pub(crate) decoder: VideoDecodeWorker,
    pub(crate) latest_frame: Option<(u64, DecodedFrame)>,
    next_frame_id: u64,
    pub(crate) received_packet: bool,
    last_stats_report: Instant,
    stats: VideoStats,
    decoder_config: DecoderConfig,
}

impl VideoReceiver {
    pub(crate) fn new(
        config: DecoderConfig,
        direct_output: Arc<DirectVideoOutput>,
    ) -> Result<Self> {
        Ok(Self {
            track_id: None,
            receiver_id: None,
            ssrc: None,
            rtp: rtp::VideoRtp::new(config.decode_width, config.decode_height),
            order: Default::default(),
            decoder: VideoDecodeWorker::spawn(config, direct_output)?,
            latest_frame: None,
            next_frame_id: 0,
            received_packet: false,
            last_stats_report: Instant::now(),
            stats: VideoStats::default(),
            decoder_config: config,
        })
    }

    pub(crate) fn open(
        &mut self,
        track_id: MediaStreamTrackId,
        receiver_id: RTCRtpReceiverId,
        ssrc: u32,
    ) {
        if self.ssrc != Some(ssrc) {
            self.order = Default::default();
        }
        self.track_id = Some(track_id);
        self.receiver_id = Some(receiver_id);
        self.ssrc = Some(ssrc);
    }

    pub(crate) fn handles(&self, track_id: &MediaStreamTrackId) -> bool {
        self.track_id.as_ref() == Some(track_id)
    }

    pub(crate) fn receive(&mut self, packet: Packet, keyframe_requested: &mut bool) {
        self.received_packet = true;
        self.stats.packets = self.stats.packets.saturating_add(1);
        let now = Instant::now();
        self.flush_order(now, keyframe_requested);
        if let Some((packet, received_at)) = self.order.push(packet.header.sequence_number, (packet, now), now) {
            self.receive_ordered(packet, received_at, keyframe_requested);
        }
        self.flush_order(now, keyframe_requested);
    }

    fn flush_order(&mut self, now: Instant, keyframe_requested: &mut bool) {
        while let Some((packet, received_at)) = self.order.pop(now) {
            self.receive_ordered(packet, received_at, keyframe_requested);
        }
    }

    fn receive_ordered(&mut self, packet: Packet, received_at: Instant, keyframe_requested: &mut bool) {
        let sample_stats = self.rtp.receive_at(&self.decoder, packet, received_at, keyframe_requested);
        macro_rules! add {
            ($field:ident) => {
                self.stats.$field = self.stats.$field.saturating_add(sample_stats.$field as u64);
            };
        }
        add!(assembled);
        add!(submitted);
        add!(dropped);
        add!(missing_sequence);
        add!(incomplete_fua);
        add!(malformed);
        add!(queue_full);
        add!(sps_rejected);
        add!(idr_wait);
        add!(other);
        add!(sequence_jumps);
        add!(estimated_missing_packets);
        add!(late_packets);
        add!(duplicate_packets);
        add!(empty_packets);
        add!(idr);
        add!(post_damage_submitted);
        if sample_stats.source_frame_duration_us.is_some() {
            self.stats.last_sample_duration_us = sample_stats.source_frame_duration_us;
        }
        if sample_stats.encoded_resolution.is_some() {
            self.stats.encoded_resolution = sample_stats.encoded_resolution;
        }
    }

    pub(crate) fn drain_decoder(&mut self, keyframe_requested: &mut bool) {
        *keyframe_requested |= self.rtp.recover_decoder(&self.decoder);
        self.flush_order(Instant::now(), keyframe_requested);
        let mut decode_errors = 0u64;
        while let Some(result) = self
            .decoder
            .latest_result
            .lock()
            .ok()
            .and_then(|mut latest| latest.take())
        {
            match result {
                Ok(frame) => {
                    self.next_frame_id = self.next_frame_id.wrapping_add(1);
                    if self.latest_frame.replace((self.next_frame_id, frame)).is_some() {
                        crate::streaming::video::metrics::METRICS
                            .handoff_replaced
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                Err(error) => {
                    eprintln!("Failed to decode H264 video frame: {error}");
                    self.stats.last_decode_error = Some(error);
                    decode_errors = decode_errors.saturating_add(1);
                    *keyframe_requested = true;
                }
            }
        }

        if decode_errors > 0 {
            self.stats.decode_errors = self.stats.decode_errors.saturating_add(decode_errors);
            // Recovery is carried by the decoder's atomic signal, not this
            // replaceable result mailbox. A delayed error must not undo a newer IDR.
        }
    }

    pub(crate) fn request_keyframe(&self, peer: &mut RTCPeerConnection) {
        // A PLI needs both identifiers recorded when the remote video track was opened.
        if let (Some(receiver_id), Some(ssrc)) = (self.receiver_id, self.ssrc)
            && let Some(mut receiver) = peer.rtp_receiver(receiver_id)
        {
            let pli = rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication {
                sender_ssrc: 0,
                media_ssrc: ssrc,
            };
            let _ = receiver.write_rtcp(vec![Box::new(pli)]);
        }
    }

    pub(crate) fn status(&mut self, now: Instant) -> Option<String> {
        if now.duration_since(self.last_stats_report) < STREAM_STATS_INTERVAL {
            return None;
        }
        self.last_stats_report = now;

        let performance = crate::streaming::video::video_performance_summary();
        let source_fps = self
            .stats
            .last_sample_duration_us
            .filter(|duration| *duration > 0)
            .map(|duration| 1_000_000 / duration)
            .unwrap_or(0);
        let encoded_resolution = self
            .stats
            .encoded_resolution
            .map(|(width, height)| format!("{width}x{height}"))
            .unwrap_or_else(|| "?".to_owned());
        let idr_age = self
            .rtp
            .idr_age_secs()
            .map(|age| format!("{age}s"))
            .unwrap_or_else(|| "?".to_owned());
        let config = self.decoder_config;
        let last_error = self
            .stats
            .last_decode_error
            .as_deref()
            .map(|error| format!("\nlast decode error: {error}"))
            .unwrap_or_default();
        Some(format!(
            "SPS:{encoded_resolution} decoder:{}x{} output:{}x{} source-fps:{source_fps}\n\
             RTP pk:{} jump:{}/~{} late:{} dup:{} empty:{}\n{}\n\
             AU done:{} sent:{} drop:{} seq:{} FU:{} mal:{} q:{} SPS:{} IDRwait:{} other:{}\n\
             IDR count:{} age:{idr_age} postDamageSent:{} wait:{} decoderErr:{}\n\
             {performance}{last_error}",
            config.decode_width,
            config.decode_height,
            config.output_width,
            config.output_height,
            self.stats.packets,
            self.order.stats.jumps,
            self.order.stats.provisional_missing,
            self.order.stats.out_of_order,
            self.order.stats.duplicates,
            self.stats.empty_packets,
            self.order.summary(),
            self.stats.assembled,
            self.stats.submitted,
            self.stats.dropped,
            self.stats.missing_sequence,
            self.stats.incomplete_fua,
            self.stats.malformed,
            self.stats.queue_full,
            self.stats.sps_rejected,
            self.stats.idr_wait,
            self.stats.other,
            self.stats.idr,
            self.stats.post_damage_submitted,
            u8::from(self.rtp.waiting_for_keyframe()),
            self.stats.decode_errors,
        ))
    }
}

pub(crate) struct AudioReceiver {
    track_id: Option<MediaStreamTrackId>,
    rtp: rtp::AudioRtp,
    pub(crate) packets: Vec<Bytes>,
}

impl AudioReceiver {
    pub(crate) fn new(sample_rate: u32, payload_type: u8) -> Self {
        Self {
            track_id: None,
            rtp: rtp::AudioRtp::new(sample_rate, payload_type),
            packets: Vec::new(),
        }
    }

    pub(crate) fn open(&mut self, track_id: MediaStreamTrackId) {
        self.track_id = Some(track_id);
    }

    pub(crate) fn handles(&self, track_id: &MediaStreamTrackId) -> bool {
        self.track_id.as_ref() == Some(track_id)
    }

    pub(crate) fn receive(&mut self, packet: Packet) {
        self.rtp.receive(packet, &mut self.packets);
    }
}
