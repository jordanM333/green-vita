//! Exercise admission at the public API, across the real ICE/DTLS/SCTP pipeline.
use bytes::BytesMut;
use rtc::data_channel::{RTCDataChannelInit, RTCDataChannelState};
use rtc::peer_connection::{RTCPeerConnection, RTCPeerConnectionBuilder};
use rtc::peer_connection::message::RTCMessage;
use rtc::peer_connection::transport::{CandidateConfig, CandidateHostConfig, RTCIceCandidate};
use rtc::sansio::Protocol;
use rtc::shared::{TaggedBytesMut, TransportContext, TransportProtocol};
use std::{net::SocketAddr, time::{Duration, Instant}};
const INPUT_OUTSTANDING_LIMIT: usize = 256; // RX Test 33 baseline policy
const FEEDBACK_OUTSTANDING_LIMIT: usize = 128;

const CHANNEL: u16 = 1;
struct Pair {
    a: RTCPeerConnection,
    b: RTCPeerConnection,
    a_addr: SocketAddr,
    b_addr: SocketAddr,
}

impl Pair {
    fn new() -> Self {
        let a_addr = "127.0.0.1:42000".parse().unwrap();
        let b_addr = "127.0.0.1:42001".parse().unwrap();
        let mut a = Self::peer(a_addr);
        let mut b = Self::peer(b_addr);
        let offer = a.create_offer(None).unwrap();
        a.set_local_description(offer.clone()).unwrap();
        b.set_remote_description(offer).unwrap();
        let answer = b.create_answer(None).unwrap();
        b.set_local_description(answer.clone()).unwrap();
        a.set_remote_description(answer).unwrap();
        Self { a, b, a_addr, b_addr }
    }

    fn peer(address: SocketAddr) -> RTCPeerConnection {
        let mut media = rtc::peer_connection::configuration::media_engine::MediaEngine::default();
        media.register_default_codecs().unwrap();
        let mut pc = RTCPeerConnectionBuilder::new().with_media_engine(media).build().unwrap();
        let candidate = CandidateHostConfig { base_config: CandidateConfig {
            network: "udp".into(), address: address.ip().to_string(),
            port: address.port(), component: 1, ..Default::default()
        }, ..Default::default() }.new_candidate_host().unwrap();
        pc.add_local_candidate(RTCIceCandidate::from(&candidate).to_json().unwrap()).unwrap();
        pc.create_data_channel("input", Some(RTCDataChannelInit {
            negotiated: Some(CHANNEL), ordered: false, max_retransmits: Some(0),
            ..Default::default()
        })).unwrap();
        pc
    }

    fn transfer(from: &mut RTCPeerConnection, to: &mut RTCPeerConnection,
        from_addr: SocketAddr, to_addr: SocketAddr, discard: bool) {
        while let Some(msg) = from.poll_write() {
            if !discard {
                to.handle_read(TaggedBytesMut { now: Instant::now(),
                    transport: TransportContext { local_addr: to_addr, peer_addr: from_addr,
                        ecn: None, transport_protocol: TransportProtocol::UDP },
                    message: msg.message }).unwrap();
            }
        }
        while from.poll_event().is_some() {}
        let now = Instant::now();
        if from.poll_timeout().is_some_and(|at| at <= now) { from.handle_timeout(now).unwrap(); }
    }

    fn tick(&mut self, drop_ack: bool) {
        Self::transfer(&mut self.a, &mut self.b, self.a_addr, self.b_addr, false);
        Self::transfer(&mut self.b, &mut self.a, self.b_addr, self.a_addr, drop_ack);
        std::thread::sleep(Duration::from_millis(1));
    }

    fn wait_for(&mut self, mut condition: impl FnMut(&mut Self) -> bool) {
        let start = Instant::now();
        while !condition(self) {
            assert!(start.elapsed() < Duration::from_secs(10), "WebRTC condition timed out");
            self.tick(false);
        }
    }
}

#[test]
fn limit_counts_ingress_and_sctp_until_ack_then_accepts_fresh_state() {
    let mut pair = Pair::new();
    assert!(pair.a.data_channel(CHANNEL).unwrap().outstanding_payload_bytes().is_err());
    pair.wait_for(|p| p.a.data_channel(CHANNEL).unwrap().ready_state() == RTCDataChannelState::Open
        && p.b.data_channel(CHANNEL).unwrap().ready_state() == RTCDataChannelState::Open);
    pair.wait_for(|p| p.a.data_channel(CHANNEL).unwrap().outstanding_payload_bytes().is_ok_and(|n| n == 0));
    {
        let mut dc = pair.a.data_channel(CHANNEL).unwrap();
        // Multiple writes before poll_write must not bypass the limit.
        for _ in 0..2 {
            assert!(dc.try_send_with_outstanding_limit(BytesMut::from(&[1; 43][..]), FEEDBACK_OUTSTANDING_LIMIT).unwrap());
        }
        assert!(!dc.try_send_with_outstanding_limit(BytesMut::from(&[2; 43][..]), FEEDBACK_OUTSTANDING_LIMIT).unwrap());
        // Feedback saturation must leave capacity for controller reports/releases.
        for _ in 0..4 {
            assert!(dc.try_send_with_outstanding_limit(BytesMut::from(&[1; 38][..]), INPUT_OUTSTANDING_LIMIT).unwrap());
        }
        assert_eq!(dc.outstanding_payload_bytes().unwrap(), 238);
        assert!(!dc.try_send_with_outstanding_limit(BytesMut::from(&[2; 38][..]), INPUT_OUTSTANDING_LIMIT).unwrap());
    }
    for _ in 0..20 { pair.tick(true); }
    assert_eq!(pair.a.data_channel(CHANNEL).unwrap().outstanding_payload_bytes().unwrap(), 238,
        "transmitted bytes remain counted until acknowledgement");
    pair.wait_for(|p| p.a.data_channel(CHANNEL).unwrap().outstanding_payload_bytes().unwrap() == 0);
    assert!(pair.a.data_channel(CHANNEL).unwrap()
        .try_send_with_outstanding_limit(BytesMut::from(&[3; 38][..]), INPUT_OUTSTANDING_LIMIT).unwrap());
    let mut reports = Vec::new();
    pair.wait_for(|p| {
        while let Some(RTCMessage::DataChannelMessage(_, msg)) = p.b.poll_read() { reports.push(msg.data); }
        reports.iter().any(|msg| msg[0] == 3)
    });
    assert_eq!(reports.iter().filter(|msg| msg[0] == 1).count(), 6);
    assert!(!reports.iter().any(|msg| msg[0] == 2), "rejected state must never reach the wire");
    pair.a.data_channel(CHANNEL).unwrap().close().unwrap();
    assert!(pair.a.data_channel(CHANNEL).unwrap()
        .try_send_with_outstanding_limit(BytesMut::from(&[4; 38][..]), 256).is_err());
}

#[test]
fn writable_admission_counts_unsent_ingress_without_starving_on_ack_wait() {
    let mut pair = Pair::new();
    assert!(pair.a.data_channel(CHANNEL).unwrap()
        .try_send_when_writable(BytesMut::from(&[1; 38][..]), 256).is_err());
    pair.wait_for(|p| p.a.data_channel(CHANNEL).unwrap().ready_state() == RTCDataChannelState::Open
        && p.b.data_channel(CHANNEL).unwrap().ready_state() == RTCDataChannelState::Open);
    pair.wait_for(|p| p.a.data_channel(CHANNEL).unwrap().outstanding_payload_bytes().is_ok_and(|n| n == 0));
    {
        let mut dc = pair.a.data_channel(CHANNEL).unwrap();
        for _ in 0..2 { assert!(dc.try_send_when_writable(BytesMut::from(&[1; 43][..]), 128).unwrap()); }
        assert!(!dc.try_send_when_writable(BytesMut::from(&[2; 43][..]), 128).unwrap());
        for _ in 0..4 { assert!(dc.try_send_when_writable(BytesMut::from(&[1; 38][..]), 256).unwrap()); }
        assert!(!dc.try_send_when_writable(BytesMut::from(&[2; 38][..]), 256).unwrap());
    }
    pair.tick(true);
    // These 238 bytes have left for the peer. An absent ACK is not a reason
    // to reject fresh state while the actual transport still has capacity.
    assert_eq!(pair.a.data_channel(CHANNEL).unwrap().outstanding_payload_bytes().unwrap(), 238);
    for _ in 0..20 {
        let mut dc = pair.a.data_channel(CHANNEL).unwrap();
        assert!(dc.try_send_when_writable(BytesMut::from(&[1; 38][..]), 256).unwrap());
        assert!(dc.try_send_when_writable(BytesMut::from(&[1; 43][..]), 128).unwrap());
        pair.tick(true);
    }
    let mut received = Vec::new();
    while let Some(RTCMessage::DataChannelMessage(_, msg)) = pair.b.poll_read() { received.push(msg.data); }
    assert_eq!(received.len(), 46, "fresh controls and feedback must both keep flowing");
    assert!(!received.iter().any(|m| m[0] == 2), "rejected reports must never be queued");
    // Exhaust the real congestion window without bypassing it.
    let mut blocked = false;
    for _ in 0..300 {
        let accepted = pair.a.data_channel(CHANNEL).unwrap()
            .try_send_when_writable(BytesMut::from(&[1; 38][..]), 256).unwrap();
        if !accepted { blocked = true; break; }
        pair.tick(true);
    }
    assert!(blocked, "admission must honor SCTP congestion control");
    pair.wait_for(|p| p.a.data_channel(CHANNEL).unwrap()
        .try_send_when_writable(BytesMut::from(&[3; 38][..]), 256).unwrap());
    pair.wait_for(|p| {
        while let Some(RTCMessage::DataChannelMessage(_, msg)) = p.b.poll_read() { received.push(msg.data); }
        received.iter().any(|m| m[0] == 3)
    });
    assert!(!received.iter().any(|m| m[0] == 2));
}

fn voice_pair() -> (Pair, crate::mic_state::Microphone, crate::mic_uplink::MicrophoneUplink, rtc::rtp_transceiver::RTCRtpSenderId) {
    use crate::{mic_state::Microphone, mic_uplink::MicrophoneUplink};
    use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
    let a_addr="127.0.0.1:42100".parse().unwrap();
    let b_addr="127.0.0.1:42101".parse().unwrap();
    let mut a=Pair::peer(a_addr); let mut b=Pair::peer(b_addr);
    a.add_transceiver_from_kind(RtpCodecKind::Video,None).unwrap();
    a.add_transceiver_from_kind(RtpCodecKind::Audio,None).unwrap();
    b.add_transceiver_from_kind(RtpCodecKind::Video,None).unwrap();
    b.add_transceiver_from_kind(RtpCodecKind::Audio,None).unwrap();
    let game_audio = b.add_track(rtc::media_stream::MediaStreamTrack::new(
        "game".into(), "game-audio".into(), "Game audio".into(), RtpCodecKind::Audio,
        vec![rtc::rtp_transceiver::rtp_sender::RTCRtpEncodingParameters {
            rtp_coding_parameters: rtc::rtp_transceiver::rtp_sender::RTCRtpCodingParameters {
                ssrc: Some(12345), ..Default::default()
            },
            codec: rtc::rtp_transceiver::rtp_sender::RTCRtpCodec {
                mime_type: "audio/opus".into(), clock_rate: 48_000, channels: 2,
                sdp_fmtp_line: "minptime=10;useinbandfec=1;stereo=1".into(), rtcp_feedback: vec![],
            }, ..Default::default()
        }],
    )).unwrap();
    let mic=Microphone::default();
    let mut uplink=MicrophoneUplink::new(&mut a,mic.clone()).unwrap();
    let offer=a.create_offer(None).unwrap();
    assert_eq!(offer.sdp.matches("m=audio").count(),1);
    assert_eq!(offer.sdp.matches("m=video").count(),1);
    assert!(!offer.sdp.contains("greenvita-voice"));
    assert!(!offer.sdp.contains("a=sendrecv"));
    assert!(offer.sdp.contains("opus/48000/2"));
    a.set_local_description(offer.clone()).unwrap();
    b.set_remote_description(offer).unwrap();
    let answer=b.create_answer(None).unwrap();
    b.set_local_description(answer.clone()).unwrap();
    a.set_remote_description(answer).unwrap();
    let mut pair=Pair{a,b,a_addr,b_addr};
    pair.wait_for(|p|p.a.data_channel(CHANNEL).unwrap().ready_state()==RTCDataChannelState::Open && p.b.data_channel(CHANNEL).unwrap().ready_state()==RTCDataChannelState::Open);
    uplink.pump(&mut pair.a,true);
    assert!(mic.available()); assert!(!mic.is_on());
    (pair, mic, uplink, game_audio)
}

fn assert_game_audio(pair: &mut Pair, sender: rtc::rtp_transceiver::RTCRtpSenderId, sequence: u16) {
    pair.b.rtp_sender(sender).unwrap().write_rtp(rtc::rtp::Packet {
        header: rtc::rtp::Header { version: 2, payload_type: 111, ssrc: 12345,
            sequence_number: sequence, timestamp: u32::from(sequence) * 960, ..Default::default() },
        payload: bytes::Bytes::from_static(&[0xf8,0xff,0xfe]),
    }).unwrap();
    pair.wait_for(|p| {
        while let Some(message) = p.a.poll_read() {
            if let RTCMessage::RtpPacket(_, packet) = message {
                if packet.header.ssrc == 12345 && packet.header.sequence_number == sequence { return true; }
            }
        }
        false
    });
}

fn assert_controls(pair: &mut Pair) {
    pair.a.data_channel(CHANNEL).unwrap().send(BytesMut::from(&b"controls"[..])).unwrap();
    pair.wait_for(|p| {
        while let Some(message) = p.b.poll_read() {
            if let RTCMessage::DataChannelMessage(_, message) = message {
                if message.data.as_ref() == b"controls" { return true; }
            }
        }
        false
    });
}

#[test]
fn microphone_reuses_audio_mline_and_mute_blocks_rtp_over_real_dtls() {
    use crate::mic_state::VoiceClip;
    let (mut pair, mic, mut uplink, game_audio) = voice_pair();
    assert_game_audio(&mut pair, game_audio, 1);
    mic.set_on(true);
    let offer = uplink.begin_negotiation(&mut pair.a).unwrap();
    assert_eq!(offer.sdp.matches("m=audio").count(), 1);
    assert_eq!(offer.sdp.matches("m=video").count(), 1);
    assert!(offer.sdp.contains("a=sendrecv"));
    assert!(offer.sdp.contains("greenvita-voice"));
    assert!(mic.begin_capture().is_none(), "no voice before Xbox accepts chat");
    assert!(uplink.begin_negotiation(&mut pair.a).is_none(), "only one exchange at a time");
    // Existing media and controller traffic must flow while chat is pending.
    assert_controls(&mut pair);
    assert_game_audio(&mut pair, game_audio, 2);
    pair.b.set_remote_description(offer).unwrap();
    let answer = pair.b.create_answer(None).unwrap();
    pair.b.set_local_description(answer.clone()).unwrap();
    // Muting during HTTP negotiation must survive its late answer.
    mic.set_on(false);
    uplink.finish_negotiation(&mut pair.a, Ok(answer.sdp));
    assert!(!mic.is_on()); assert!(mic.begin_capture().is_none());
    assert_controls(&mut pair);
    assert_game_audio(&mut pair, game_audio, 3);
    let mut received=Vec::new();
    let drain=|p:&mut Pair, received:&mut Vec<rtc::rtp::Packet>| {
        while let Some(message)=p.b.poll_read() {
            if let RTCMessage::RtpPacket(_,packet)=message { received.push(packet); }
        }
    };
    mic.set_on(true);
    assert!(uplink.begin_negotiation(&mut pair.a).is_none(), "unmute reuses accepted chat");
    let make_clip=|ticket,ts|VoiceClip{ticket,captured_at:Instant::now(),timestamp:ts,opus:vec![0xf8,0xff,0xfe]};
    let stale=mic.begin_capture().unwrap();
    mic.publish(make_clip(stale,1000),0.5);
    mic.set_on(false);
    uplink.pump(&mut pair.a,true);
    for _ in 0..10 {pair.tick(false);}
    drain(&mut pair,&mut received); assert!(received.is_empty());
    mic.set_on(true);
    mic.publish(make_clip(stale,1960),0.5); // Late callback from before mute.
    mic.publish(make_clip(mic.begin_capture().unwrap(),2920),0.5);
    uplink.pump(&mut pair.a,true);
    pair.wait_for(|p| {drain(p,&mut received); !received.is_empty()});
    assert_eq!(received.len(),1); assert_eq!(received[0].header.timestamp,2920);
    assert_eq!(received[0].payload.as_ref(),[0xf8,0xff,0xfe]);
    mic.publish(make_clip(mic.begin_capture().unwrap(),3880),0.5);
    uplink.pump(&mut pair.a,true);
    pair.wait_for(|p| {drain(p,&mut received); received.len()==2});
    assert_eq!(received[1].header.timestamp.wrapping_sub(received[0].header.timestamp),960);
    assert_eq!(received[1].header.sequence_number,received[0].header.sequence_number.wrapping_add(1));
    uplink.pump(&mut pair.a,false);
    assert!(!mic.is_on()); assert!(!mic.available());
    uplink.pump(&mut pair.a,true); assert!(!mic.is_on());
}

#[test]
fn failed_chat_exchange_rolls_back_without_stopping_game_media() {
    let (mut pair, mic, mut uplink, game_audio) = voice_pair();
    mic.set_on(true);
    assert!(uplink.begin_negotiation(&mut pair.a).is_some());
    uplink.finish_negotiation(&mut pair.a, Err(anyhow::anyhow!("chat request timed out")));
    assert!(pair.a.pending_local_description().is_none());
    assert!(!mic.is_on()); assert!(!mic.available()); assert!(mic.begin_capture().is_none());
    assert!(mic.status().contains("timed out"));
    assert!(uplink.begin_negotiation(&mut pair.a).is_none());
    assert_controls(&mut pair);
    assert_game_audio(&mut pair, game_audio, 1);
}
