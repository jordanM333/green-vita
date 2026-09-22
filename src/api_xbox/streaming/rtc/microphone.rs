//! Opt-in voice packets on the negotiated Opus sender (not the chat data channel).
use crate::api::streaming::rtc::peer::RTCPeerConnection;
use crate::streaming::microphone::Microphone;
use crate::streaming::microphone_capture::MicrophoneCapture;
use anyhow::{Context, Result};
use bytes::Bytes;
use rtc::media_stream::MediaStreamTrack;
use rtc::rtp_transceiver::{RTCRtpSenderId, RTCRtpTransceiverId};
use rtc::rtp_transceiver::rtp_sender::{RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind};

pub(super) struct MicrophoneUplink {
    microphone: Microphone,
    _capture: Option<MicrophoneCapture>,
    capture_error: Option<String>,
    sender: RTCRtpSenderId,
    transceiver: RTCRtpTransceiverId,
    ssrc: u32,
    sequence: u16,
    previous_timestamp: Option<u32>,
    connected: bool,
}
impl MicrophoneUplink {
    pub(super) fn new(peer: &mut RTCPeerConnection, microphone: Microphone) -> Result<Self> {
        let random = *uuid::Uuid::new_v4().as_bytes();
        let ssrc = u32::from_ne_bytes(random[..4].try_into().unwrap());
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
        // add_track reuses the existing receive-only audio m-line. Opus's /2
        // SDP mapping supports mono voice while preserving stereo game audio.
        let ids = peer.get_transceivers().collect::<Vec<_>>();
        let transceiver = ids.into_iter().find(|id| peer.rtp_transceiver(*id)
            .is_some_and(|t| t.sender() == Some(sender))).context("microphone transceiver")?;
        // Failure to start optional voice capture must not stop game streaming.
        let (capture, capture_error) = match MicrophoneCapture::spawn(microphone.clone()) {
            Ok(capture) => (Some(capture), None),
            Err(error) => (None, Some(format!("capture worker: {error}"))),
        };
        Ok(Self { microphone, _capture: capture, capture_error, sender, transceiver, ssrc,
            sequence: u16::from_ne_bytes(random[4..6].try_into().unwrap()),
            previous_timestamp: None, connected: false })
    }

    pub(super) fn pump(&mut self, peer: &mut RTCPeerConnection, connected: bool) {
        if connected != self.connected {
            self.connected = connected;
            let accepted = connected && peer.rtp_transceiver(self.transceiver)
                .is_some_and(|t| t.current_direction().has_send());
            self.microphone.set_ready(accepted && self._capture.is_some());
            if connected && let Some(error) = &self.capture_error {
                self.microphone.fail(error.clone());
            } else if connected && !accepted {
                self.microphone.fail("Xbox did not accept the audio uplink".into());
            }
        }
        if !connected { return; }
        let Some(mut sender) = peer.rtp_sender(self.sender) else { return; };
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
