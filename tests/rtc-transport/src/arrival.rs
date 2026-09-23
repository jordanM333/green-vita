//! Real ICE/DTLS/SRTP/SRTCP negotiation, with decrypted feedback observed at
//! the sender interceptor. No clear-header inference stands in for delivery.
use rtc::interceptor::{Interceptor, NoopInterceptor, Packet, Registry, StreamInfo, TaggedPacket};
use rtc::peer_connection::{RTCPeerConnection, RTCPeerConnectionBuilder};
use rtc::peer_connection::configuration::media_engine::MediaEngine;
use rtc::peer_connection::transport::{CandidateConfig, CandidateHostConfig, RTCIceCandidate};
use rtc::rtp_transceiver::rtp_sender::{RTCPFeedback, RTCRtpCodec, RTCRtpCodecParameters,
    RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind};
use rtc::rtcp::transport_feedbacks::transport_layer_cc::TransportLayerCc;
use rtc::sansio::Protocol;
use rtc::shared::{TaggedBytesMut, TransportContext, TransportProtocol};
use std::{net::SocketAddr, sync::{Arc, Mutex}, time::{Duration, Instant}};
use crate::arrival_feedback::{self, TRANSPORT_CC_URI};

#[derive(Default)]
struct Seen { twcc: Vec<TransportLayerCc>, rr: usize }
struct Observe { inner: NoopInterceptor, seen: Arc<Mutex<Seen>> }
impl Protocol<TaggedPacket, TaggedPacket, ()> for Observe {
    type Rout = TaggedPacket; type Wout = TaggedPacket; type Eout = ();
    type Error = rtc::shared::error::Error; type Time = Instant;
    fn handle_read(&mut self, msg: TaggedPacket) -> Result<(), Self::Error> {
        if let Packet::Rtcp(packets) = &msg.message {
            let mut seen = self.seen.lock().unwrap();
            for p in packets {
                if let Some(tcc) = p.as_any().downcast_ref::<TransportLayerCc>() { seen.twcc.push(tcc.clone()); }
                if p.as_any().is::<rtc::rtcp::receiver_report::ReceiverReport>() { seen.rr += 1; }
            }
        }
        self.inner.handle_read(msg)
    }
    fn poll_read(&mut self) -> Option<Self::Rout> { self.inner.poll_read() }
    fn handle_write(&mut self, msg: TaggedPacket) -> Result<(), Self::Error> { self.inner.handle_write(msg) }
    fn poll_write(&mut self) -> Option<Self::Wout> { self.inner.poll_write() }
    fn handle_event(&mut self, e: ()) -> Result<(), Self::Error> { self.inner.handle_event(e) }
    fn poll_event(&mut self) -> Option<()> { self.inner.poll_event() }
    fn handle_timeout(&mut self, now: Instant) -> Result<(), Self::Error> { self.inner.handle_timeout(now) }
    fn poll_timeout(&mut self) -> Option<Instant> { self.inner.poll_timeout() }
    fn close(&mut self) -> Result<(), Self::Error> { self.inner.close() }
}
impl Interceptor for Observe {
    fn bind_local_stream(&mut self, info: &StreamInfo) { self.inner.bind_local_stream(info); }
    fn unbind_local_stream(&mut self, info: &StreamInfo) { self.inner.unbind_local_stream(info); }
    fn bind_remote_stream(&mut self, info: &StreamInfo) { self.inner.bind_remote_stream(info); }
    fn unbind_remote_stream(&mut self, info: &StreamInfo) { self.inner.unbind_remote_stream(info); }
}

fn candidate<I: Interceptor>(pc: &mut RTCPeerConnection<I>, address: SocketAddr) {
    let c = CandidateHostConfig { base_config: CandidateConfig {
        network: "udp".into(), address: address.ip().to_string(), port: address.port(),
        component: 1, ..Default::default()
    }, ..Default::default() }.new_candidate_host().unwrap();
    pc.add_local_candidate(RTCIceCandidate::from(&c).to_json().unwrap()).unwrap();
}
fn transfer<I: Interceptor, J: Interceptor>(a: &mut RTCPeerConnection<I>, b: &mut RTCPeerConnection<J>,
    a_addr: SocketAddr, b_addr: SocketAddr) {
    let now = Instant::now();
    if a.poll_timeout().is_some_and(|at| at <= now) { a.handle_timeout(now).unwrap(); }
    while let Some(msg) = a.poll_write() {
        b.handle_read(TaggedBytesMut { now: Instant::now(), transport: TransportContext {
            local_addr: b_addr, peer_addr: a_addr, ecn: None, transport_protocol: TransportProtocol::UDP
        }, message: msg.message }).unwrap();
    }
    while a.poll_event().is_some() {}
}
fn codec() -> RTCRtpCodec {
    RTCRtpCodec { mime_type: "video/H264".into(), clock_rate: 90_000,
        sdp_fmtp_line: "packetization-mode=1;profile-level-id=42001f".into(),
        rtcp_feedback: vec![RTCPFeedback { typ: "nack".into(), ..Default::default() }],
        ..Default::default() }
}
fn media() -> MediaEngine {
    let mut m = MediaEngine::default();
    m.register_codec(RTCRtpCodecParameters { rtp_codec: codec(), payload_type: 102 }, RtpCodecKind::Video).unwrap();
    m
}

fn encrypted_feedback(accept: bool) {
    let mut receive_media = media();
    let registry = arrival_feedback::configure(&mut receive_media, vec![(102,90_000)]).unwrap();
    let mut receiver = RTCPeerConnectionBuilder::new().with_media_engine(receive_media)
        .with_interceptor_registry(registry).build().unwrap();
    let mut send_media = media();
    if accept { let _ = arrival_feedback::configure(&mut send_media, vec![]).unwrap(); }
    let seen = Arc::new(Mutex::new(Seen::default()));
    let observed = seen.clone();
    let mut sender = RTCPeerConnectionBuilder::new().with_media_engine(send_media)
        .with_interceptor_registry(Registry::new().with(move |inner| Observe { inner, seen: observed }))
        .build().unwrap();
    let r_addr = "127.0.0.1:42300".parse().unwrap();
    let s_addr = "127.0.0.1:42301".parse().unwrap();
    candidate(&mut receiver, r_addr); candidate(&mut sender, s_addr);
    receiver.add_transceiver_from_kind(RtpCodecKind::Video, None).unwrap();
    let video = sender.add_track(rtc::media_stream::MediaStreamTrack::new(
        "game".into(), "video".into(), "Game".into(), RtpCodecKind::Video,
        vec![RTCRtpEncodingParameters { rtp_coding_parameters: RTCRtpCodingParameters {
            ssrc: Some(12345), ..Default::default()
        }, codec: codec(), ..Default::default() }],
    )).unwrap();
    let offer = receiver.create_offer(None).unwrap();
    assert!(offer.sdp.contains("transport-cc"));
    assert!(offer.sdp.contains(TRANSPORT_CC_URI));
    let extension = offer.sdp.lines().find(|l| l.starts_with("a=extmap:") && l.contains(TRANSPORT_CC_URI)).unwrap()
        .split_whitespace().next().unwrap().trim_start_matches("a=extmap:").parse::<u8>().unwrap();
    receiver.set_local_description(offer.clone()).unwrap(); sender.set_remote_description(offer).unwrap();
    let answer = sender.create_answer(None).unwrap();
    assert_eq!(answer.sdp.contains(TRANSPORT_CC_URI), accept);
    assert_eq!(answer.sdp.contains("transport-cc"), accept);
    assert!(answer.sdp.contains("a=ssrc:12345")); // exercise the formerly missing binding
    sender.set_local_description(answer.clone()).unwrap(); receiver.set_remote_description(answer).unwrap();
    let start = Instant::now();
    let mut sequence = 0u16;
    let mut next = start + Duration::from_millis(300);
    let mut delivered = 0;
    while start.elapsed() < Duration::from_millis(1400) {
        transfer(&mut receiver, &mut sender, r_addr, s_addr);
        transfer(&mut sender, &mut receiver, s_addr, r_addr);
        if Instant::now() >= next && sequence < 20 {
            let mut header = rtc::rtp::Header { version: 2, ssrc: 12345, payload_type: 102,
                sequence_number: sequence, timestamp: u32::from(sequence)*1500, ..Default::default() };
            // Also inject the extension in the declined case: a stray header
            // alone must not enable unnegotiated feedback.
            header.set_extension(extension, bytes::Bytes::copy_from_slice(&sequence.wrapping_add(65530).to_be_bytes())).unwrap();
            sender.rtp_sender(video).unwrap().write_rtp(rtc::rtp::Packet {
                header, payload: bytes::Bytes::from_static(&[0x41, 1, 2, 3])
            }).unwrap();
            sequence += 1; next += Duration::from_millis(10);
        }
        while let Some(msg) = receiver.poll_read() {
            if let rtc::peer_connection::message::RTCMessage::RtpPacket(_, _) = msg { delivered += 1; }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(delivered, 20);
    let seen = seen.lock().unwrap();
    assert!(seen.rr > 0, "existing receiver reports must still cross SRTCP");
    if accept {
        assert!(!seen.twcc.is_empty(), "negotiated arrival feedback must be decrypted at the remote sender");
        assert!(seen.twcc.iter().all(|p| p.media_ssrc == 12345));
        assert!(seen.twcc.iter().map(|p| p.recv_deltas.len()).sum::<usize>() >= 19);
    } else { assert!(seen.twcc.is_empty(), "peer declined the extension"); }
}
#[test]
fn negotiated_arrival_feedback_crosses_srtcp_after_first_packet_codec_discovery() { encrypted_feedback(true); }
#[test]
fn declined_arrival_feedback_keeps_media_and_receiver_reports_working() { encrypted_feedback(false); }

#[test]
fn arrival_binding_after_clock_fallback_preserves_loss_and_reports_wraparound() {
    use rtc::interceptor::{RTPHeaderExtension, TwccReceiverBuilder};
    let mut chain = TwccReceiverBuilder::new().build()(crate::reports::ReceiveReports::new(
        NoopInterceptor::new(), vec![(102,90_000)]));
    let now = Instant::now();
    let packet = |seq: u16, transport_seq: u16, at| {
        let mut header = rtc::rtp::Header { version: 2, ssrc: 10, payload_type: 102,
            sequence_number: seq, timestamp: u32::from(seq)*1500, ..Default::default() };
        header.set_extension(5, bytes::Bytes::copy_from_slice(&transport_seq.to_be_bytes())).unwrap();
        TaggedPacket { now: at, transport: Default::default(), message: Packet::Rtp(rtc::rtp::Packet {
            header, payload: bytes::Bytes::from_static(&[0x41])
        }) }
    };
    chain.handle_read(packet(100,65532,now)).unwrap(); chain.poll_read();
    let info = StreamInfo { ssrc: 10, clock_rate: 90_000, payload_type: 102,
        rtp_header_extensions: vec![RTPHeaderExtension { uri: TRANSPORT_CC_URI.into(), id: 5 }],
        ..Default::default() };
    chain.bind_remote_stream(&info);
    for (seq, transport_seq, ms) in [(102,65534,10), (103,0,30), (104,1,40)] {
        chain.handle_read(packet(seq,transport_seq,now+Duration::from_millis(ms))).unwrap(); chain.poll_read();
    }
    chain.bind_remote_stream(&info); // renegotiation must not reset RR or arrival history
    chain.handle_timeout(now + Duration::from_secs(2)).unwrap();
    let mut rr = false; let mut twcc = false;
    while let Some(msg) = chain.poll_write() {
        if let Packet::Rtcp(packets) = msg.message { for p in packets {
            if let Some(r) = p.as_any().downcast_ref::<rtc::rtcp::receiver_report::ReceiverReport>() {
                assert_eq!(r.reports[0].total_lost, 1); assert_eq!(r.reports[0].last_sequence_number,104); rr=true;
            }
            if let Some(t) = p.as_any().downcast_ref::<TransportLayerCc>() {
                assert_eq!(t.base_sequence_number,65534); assert_eq!(t.packet_status_count,4);
                assert_eq!(t.recv_deltas.len(),3);
                assert_eq!(t.recv_deltas[1].delta,20_000); assert_eq!(t.recv_deltas[2].delta,10_000);
                twcc=true;
            }
        } }
    }
    assert!(rr && twcc);
    chain.unbind_remote_stream(&info);
    chain.handle_read(packet(105,2,now+Duration::from_secs(3))).unwrap(); chain.poll_read();
    chain.handle_timeout(now+Duration::from_secs(4)).unwrap();
    while let Some(msg)=chain.poll_write() {
        if let Packet::Rtcp(packets)=msg.message { assert!(!packets.iter().any(|p| p.as_any().is::<TransportLayerCc>())); }
    }
}
