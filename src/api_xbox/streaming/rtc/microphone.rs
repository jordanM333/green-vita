//! Opt-in voice packets on the negotiated Opus sender (not the chat data channel).
use crate::api::streaming::rtc::peer::RTCPeerConnection;
use crate::streaming::microphone::Microphone;
use crate::streaming::microphone_capture::MicrophoneCapture;
use anyhow::{Context, Result, ensure};
use rtc::peer_connection::sdp::RTCSessionDescription;
use bytes::Bytes;
use rtc::media_stream::MediaStreamTrack;
use rtc::rtp_transceiver::{RTCRtpSenderId, RTCRtpTransceiverId};
use rtc::rtp_transceiver::rtp_sender::{RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind};

pub(super) struct MicrophoneUplink {
    microphone: Microphone,
    _capture: Option<MicrophoneCapture>,
    capture_error: Option<String>,
    sender: Option<RTCRtpSenderId>,
    transceiver: Option<RTCRtpTransceiverId>,
    negotiated: bool,
    pending: bool,
    failed: bool,
    ssrc: u32,
    sequence: u16,
    previous_timestamp: Option<u32>,
    connected: bool,
}
impl MicrophoneUplink {
    pub(super) fn new(_peer: &mut RTCPeerConnection, microphone: Microphone) -> Result<Self> {
        let random = *uuid::Uuid::new_v4().as_bytes();
        let ssrc = u32::from_ne_bytes(random[..4].try_into().unwrap());
        // Initial streaming SDP stays receive-only. Xbox chat is negotiated on
        // the first explicit unmute, through its separate chat SDP exchange.
        let (capture, capture_error) = match MicrophoneCapture::spawn(microphone.clone()) {
            Ok(capture) => (Some(capture), None),
            Err(error) => (None, Some(format!("capture worker: {error}"))),
        };
        Ok(Self { microphone, _capture: capture, capture_error, sender: None, transceiver: None,
            negotiated: false, pending: false, failed: false, ssrc,
            sequence: u16::from_ne_bytes(random[4..6].try_into().unwrap()),
            previous_timestamp: None, connected: false })
    }

    pub(super) fn status(&self) -> String { self.microphone.status() }

    pub(super) fn begin_negotiation(&mut self, peer: &mut RTCPeerConnection) -> Option<RTCSessionDescription> {
        if !self.connected || !self.microphone.is_on() || self.negotiated || self.pending || self.failed {
            return None;
        }
        match self.create_chat_offer(peer) {
            Ok(offer) => { self.pending = true; Some(offer) }
            Err(error) => { self.finish_negotiation(peer, Err(error)); None }
        }
    }

    fn create_chat_offer(&mut self, peer: &mut RTCPeerConnection) -> Result<RTCSessionDescription> {
        let ssrc = self.ssrc;
        let sender = peer.add_track(MediaStreamTrack::new(
            "greenvita-voice".into(), "microphone".into(), "Vita microphone".into(), RtpCodecKind::Audio,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters { ssrc: Some(ssrc), ..Default::default() },
                codec: RTCRtpCodec {
                    mime_type: "audio/opus".into(), clock_rate: 48_000, channels: 2,
                    sdp_fmtp_line: "minptime=10;useinbandfec=1;stereo=1".into(),
                    rtcp_feedback: vec![],
                }, ..Default::default()
            }],
        )).context("add microphone track")?;
        self.sender = Some(sender);
        // add_track reuses the existing receive-only audio m-line. Opus's /2
        // SDP mapping supports mono voice while preserving stereo game audio.
        let ids = peer.get_transceivers().collect::<Vec<_>>();
        let transceiver = ids.into_iter().find(|id| peer.rtp_transceiver(*id)
            .is_some_and(|t| t.sender() == Some(sender))).context("microphone transceiver")?;
        self.transceiver = Some(transceiver);
        let offer = peer.create_offer_with_video_bandwidth(None, crate::api::streaming::rtc::bandwidth::VIDEO_CEILING_BPS).context("create Xbox chat offer")?;
        peer.set_local_description(offer.clone()).context("set Xbox chat offer")?;
        Ok(offer)
    }

    pub(super) fn finish_negotiation(&mut self, peer: &mut RTCPeerConnection, answer: Result<String>) -> bool {
        self.pending = false;
        let mut applied = false;
        let result = answer.and_then(|sdp| {
            peer.set_remote_description(RTCSessionDescription::answer(sdp)?)
                .context("apply Xbox chat answer")?;
            applied = true;
            ensure!(self.transceiver.and_then(|id| peer.rtp_transceiver(id))
                .is_some_and(|t| t.current_direction().has_send()), "Xbox declined microphone audio");
            Ok(())
        });
        match result {
            Ok(()) => {
                self.negotiated = true;
                self.microphone.set_negotiated(self.connected);
            }
            Err(error) => {
                // Stop optional voice without tearing down working game media.
                if peer.pending_local_description().is_some() {
                    let rollback = RTCSessionDescription::rollback(None)
                        .and_then(|sdp| peer.set_local_description(sdp));
                    if let Err(error) = rollback { eprintln!("Chat SDP rollback failed: {error}"); }
                }
                if let Some(sender) = self.sender.take() { let _ = peer.remove_track(sender); }
                self.failed = true;
                self.negotiated = false;
                eprintln!("Xbox microphone negotiation failed: {error:#}");
                self.microphone.fail(format!("chat connection failed: {error}"));
            }
        }
        applied
    }

    pub(super) fn pump(&mut self, peer: &mut RTCPeerConnection, connected: bool) {
        if connected != self.connected {
            self.connected = connected;
            self.microphone.set_ready(connected && self._capture.is_some() && !self.failed);
            self.microphone.set_negotiated(connected && self.negotiated);
            if connected && let Some(error) = &self.capture_error {
                self.microphone.fail(error.clone());
            }
        }
        if !connected || !self.negotiated { return; }
        let Some(mut sender) = self.sender.and_then(|id| peer.rtp_sender(id)) else { return; };
        let sequence = &mut self.sequence;
        let previous = &mut self.previous_timestamp;
        let ssrc = self.ssrc;
        self.microphone.send_pending(|clip| {
            let marker = previous.is_none_or(|ts| clip.timestamp.wrapping_sub(ts) != 960);
            let packet = rtc::rtp::Packet {
                header: rtc::rtp::Header { version: 2, payload_type: 111, ssrc,
                    sequence_number: *sequence, timestamp: clip.timestamp, marker, ..Default::default() },
                payload: Bytes::copy_from_slice(&clip.opus),
            };
            if sender.write_rtp(packet).is_err() { return false; }
            *sequence = sequence.wrapping_add(1); *previous = Some(clip.timestamp);
            true
        });
    }
}
