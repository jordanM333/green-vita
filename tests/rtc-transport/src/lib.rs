//! Exercise the actual SCTP dependency with a virtual clock and two endpoints.
//! This reproduces transport queueing; it does not emulate Xbox arrival_feedback control.
#[cfg(test)]
mod delivery;
#[cfg(test)]
mod webrtc;
#[cfg(test)]
mod arrival;
#[path = "../../../src/api/streaming/rtc/reports.rs"]
mod reports;
#[path = "../../../src/api/streaming/rtc/arrival_feedback.rs"]
mod arrival_feedback;
#[path = "../../../src/api/streaming/rtc/feedback.rs"]
mod feedback;
#[path = "../../../src/api/streaming/rtc/traffic.rs"]
mod traffic;
#[path = "../../../src/api_xbox/streaming/control/admission.rs"]
pub mod admission;
#[path = "../../../src/api/streaming/rtc/latest_input.rs"]
#[cfg(test)]
mod latest_input;
#[cfg(test)]
mod tests {
    const INPUT_OUTSTANDING_LIMIT: usize = 256; // RX Test 33 baseline policy
    use bytes::Bytes;
    use sctp::{Association, AssociationHandle, ClientConfig, DatagramEvent, Endpoint,
        EndpointConfig, Payload, PayloadProtocolIdentifier, ReliabilityType, ServerConfig,
        TransportConfig};
    use shared::TransportProtocol;
    use std::{collections::VecDeque, net::SocketAddr, sync::Arc, time::{Duration, Instant}};

    struct Side {
        endpoint: Endpoint,
        address: SocketAddr,
        association: Option<(AssociationHandle, Association)>,
        incoming: VecDeque<(Instant, Bytes)>,
        received: Vec<(u64, u64)>,
    }

    impl Side {
        fn new(port: u16, server: bool) -> Self {
            let address = SocketAddr::from(([127, 0, 0, 1], port));
            Self { endpoint: Endpoint::new(address, TransportProtocol::UDP,
                Arc::new(EndpointConfig::default()), server.then(|| Arc::new(ServerConfig::default()))),
                address, association: None, incoming: VecDeque::new(), received: Vec::new() }
        }

        fn drive(&mut self, remote: &mut Self, now: Instant, origin: Instant, lose: bool) {
            while self.incoming.front().is_some_and(|(at, _)| *at <= now) {
                let (at, bytes) = self.incoming.pop_front().unwrap();
                if let Some((handle, event)) = self.endpoint.handle(at, remote.address, None, bytes) {
                    match event {
                        DatagramEvent::NewAssociation(conn) => self.association = Some((handle, conn)),
                        DatagramEvent::AssociationEvent(event) => self.association.as_mut().unwrap().1.handle_event(event),
                    }
                }
            }
            let Some((_, conn)) = self.association.as_mut() else { return; };
            if conn.poll_timeout().is_some_and(|at| at <= now) { conn.handle_timeout(now); }
            while conn.poll().is_some() {}
            for id in conn.stream_ids() {
                let mut stream = conn.stream(id).unwrap();
                while let Some(chunks) = stream.read_sctp().unwrap() {
                    let mut buf = vec![0; chunks.len()];
                    chunks.read(&mut buf).unwrap();
                    if buf.len() >= 8 {
                        self.received.push((u64::from_le_bytes(buf[..8].try_into().unwrap()),
                            now.duration_since(origin).as_millis() as u64));
                    }
                }
            }
            while let Some(tx) = conn.poll_transmit(now) {
                if let Payload::RawEncode(packets) = tx.message {
                    if !lose {
                        for packet in packets { remote.incoming.push_back((now + Duration::from_millis(5), packet)); }
                    }
                }
            }
        }
    }

    #[derive(Clone, Copy)]
    enum Admission { Unbounded, Test33AckCap, Test34Capacity }

    fn simulate(stall: bool, admission: Admission, drop_forward: bool) -> (u64, u64, usize) {
        simulate_for(stall, admission, drop_forward, 120_000)
    }

    fn simulate_for(stall: bool, admission: Admission, drop_forward: bool, duration_ms: u64) -> (u64, u64, usize) {
        let mut client = Side::new(41000, false);
        let mut server = Side::new(41001, true);
        client.association = Some(client.endpoint.connect(ClientConfig::new(TransportConfig::default()), server.address).unwrap());
        let start = Instant::now();
        let mut max_buffer = 0;
        // 120 reports/s approximates separate controller and presentation reports.
        // Stop acknowledgements for two seconds, then allow normal delivery.
        for ms in 0..duration_ms {
            let now = start + Duration::from_millis(ms);
            if ms == 1000 {
                let conn = &mut client.association.as_mut().unwrap().1;
                assert!(!conn.is_handshaking());
                conn.open_stream(2, PayloadProtocolIdentifier::Binary).unwrap()
                    .set_reliability_params(true, ReliabilityType::Rexmit, 0).unwrap();
            }
            if (1000..duration_ms - 1000).contains(&ms) && ms % 8 == 0 {
                let capacity = client.association.as_ref().unwrap().1.immediate_send_capacity();
                let mut stream = client.association.as_mut().unwrap().1.stream(2).unwrap();
                let mut bytes = vec![0; 43];
                bytes[..8].copy_from_slice(&ms.to_le_bytes());
                let admitted = match admission {
                    Admission::Unbounded => true,
                    Admission::Test33AckCap => stream.buffered_amount().unwrap() + bytes.len() <= INPUT_OUTSTANDING_LIMIT,
                    Admission::Test34Capacity => bytes.len() <= capacity,
                };
                if admitted {
                    stream.write_sctp(&Bytes::from(bytes), PayloadProtocolIdentifier::Binary).unwrap();
                }
                max_buffer = max_buffer.max(stream.buffered_amount().unwrap());
            }
            let outage = stall && ms >= 40_000 && ms % 40_000 < 2000;
            client.drive(&mut server, now, start, outage && drop_forward);
            server.drive(&mut client, now, start, outage);
        }
        let max_age = server.received.iter().map(|(sent, received)| received - sent).max().unwrap();
        let tail_age = server.received.iter().filter(|(sent, _)| *sent >= duration_ms - 10_000)
            .map(|(sent, received)| received - sent).max().unwrap();
        (max_age, tail_age, max_buffer)
    }

    #[test]
    fn thirty_virtual_minutes_of_repeated_outages_do_not_accumulate_input_age() {
        let (age, tail, outstanding) = simulate_for(true, Admission::Test34Capacity, true, 1_800_000);
        eprintln!("1800s virtual SCTP, 44 two-second outages: max={age}ms tail={tail}ms outstanding={outstanding}B");
        assert!(age < 50);
        assert!(tail < 50);
    }

    #[test]
    fn steady_unreliable_input_does_not_drift_over_two_minutes() {
        let (max_age, tail_age, max_buffer) = simulate(false, Admission::Unbounded, false);
        eprintln!("steady: max age={max_age}ms tail={tail_age}ms buffered={max_buffer}B");
        assert!(max_age < 50);
        assert!(tail_age < 50);
    }

    #[test]
    fn bounded_admission_does_not_replay_stale_reports_after_ack_stall() {
        let (max_age, tail_age, max_buffer) = simulate(true, Admission::Test33AckCap, false);
        eprintln!("bounded: max age={max_age}ms tail={tail_age}ms buffered={max_buffer}B");
        assert!(max_age < 50);
        assert!(max_buffer <= INPUT_OUTSTANDING_LIMIT);
    }

    #[test]
    fn zero_retransmits_alone_still_delivers_two_second_old_reports() {
        let (max_age, tail_age, max_buffer) = simulate(true, Admission::Unbounded, false);
        eprintln!("ack stall: max age={max_age}ms tail={tail_age}ms buffered={max_buffer}B");
        assert!(max_age >= 1900, "baseline must reproduce delayed reports");
        assert!(max_buffer > 10_000, "baseline must reproduce the hidden backlog");
        assert!(tail_age < 50, "transport did not recover after acknowledgements resumed");
    }

    #[test]
    fn bounded_reports_remain_fresh_after_repeated_bidirectional_outages() {
        let (max_age, tail_age, max_buffer) = simulate(true, Admission::Test33AckCap, true);
        eprintln!("bidirectional outages: max age={max_age}ms tail={tail_age}ms buffered={max_buffer}B");
        assert!(max_age < 50);
        assert!(tail_age < 50);
        assert!(max_buffer <= INPUT_OUTSTANDING_LIMIT);
    }

    #[test]
    fn test34_capacity_admission_does_not_replay_old_reports_after_ack_outages() {
        let (max_age, tail_age, max_buffer) = simulate(true, Admission::Test34Capacity, false);
        eprintln!("Test34 ACK outages: max age={max_age}ms tail={tail_age}ms outstanding={max_buffer}B");
        assert!(max_age < 50);
        assert!(tail_age < 50);
    }

    #[test]
    fn test34_capacity_admission_recovers_after_bidirectional_outages() {
        let (max_age, tail_age, max_buffer) = simulate(true, Admission::Test34Capacity, true);
        eprintln!("Test34 bidirectional outages: max age={max_age}ms tail={tail_age}ms outstanding={max_buffer}B");
        assert!(max_age < 50);
        assert!(tail_age < 50);
    }
}

// Compile the production mic uplink against the real RTC dependency.
#[path = "../../../src/api/streaming/rtc/bandwidth.rs"]
pub(crate) mod bandwidth;
mod api { pub mod streaming { pub mod rtc { pub(crate) use crate::bandwidth; pub mod peer {
    pub type RTCPeerConnection = ::rtc::peer_connection::RTCPeerConnection<crate::arrival_feedback::ReceiveFeedback>;
} } } }
#[path = "../../../src/streaming/microphone.rs"]
mod mic_state;
#[path = "../../../src/streaming/microphone_capture.rs"]
mod mic_capture;
mod streaming {
    pub(crate) use super::mic_state as microphone;
    // Hardware input is injected in the peer test. Production capture still
    // compiles above; the real codec has a separate native Opus roundtrip test.
    pub mod microphone_capture {
        pub struct MicrophoneCapture;
        impl MicrophoneCapture {
            pub fn spawn(_: super::microphone::Microphone) -> anyhow::Result<Self> { Ok(Self) }
        }
    }
}
use mic_state as microphone;
#[path = "../../../src/api_xbox/streaming/rtc/microphone.rs"]
mod mic_uplink;
