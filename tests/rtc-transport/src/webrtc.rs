//! Exercise admission at the public API, across the real ICE/DTLS/SCTP pipeline.
use bytes::BytesMut;
use rtc::data_channel::{RTCDataChannelInit, RTCDataChannelState};
use rtc::peer_connection::{RTCPeerConnection, RTCPeerConnectionBuilder};
use rtc::peer_connection::message::RTCMessage;
use rtc::peer_connection::transport::{CandidateConfig, CandidateHostConfig, RTCIceCandidate};
use rtc::sansio::Protocol;
use rtc::shared::{TaggedBytesMut, TransportContext, TransportProtocol};
use std::{net::SocketAddr, time::{Duration, Instant}};
use crate::admission::{INPUT_OUTSTANDING_LIMIT, FEEDBACK_OUTSTANDING_LIMIT};

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
        let mut pc = RTCPeerConnectionBuilder::new().build().unwrap();
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
