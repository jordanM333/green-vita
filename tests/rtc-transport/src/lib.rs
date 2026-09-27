//! Exercise the actual SCTP dependency with a virtual clock and two endpoints.
//! This reproduces transport queueing; it does not emulate Xbox arrival_feedback control.
#![cfg(test)]
#[allow(dead_code)]
#[path = "../../../src/diagnostic.rs"]
mod diagnostic;
#[allow(dead_code)]
#[path = "../../../src/build_info.rs"]
mod build_info;
#[path = "../../../src/api_xbox/streaming/control/admission.rs"]
pub mod admission;
#[cfg(test)]
mod arrival;
#[path = "../../../src/api/streaming/rtc/arrival_feedback.rs"]
mod arrival_feedback;
#[cfg(test)]
mod delivery;
#[path = "../../../src/api/streaming/rtc/feedback.rs"]
mod feedback;
#[path = "../../../src/api/streaming/rtc/latest_input.rs"]
#[cfg(test)]
mod latest_input;
#[path = "../../../src/api/streaming/rtc/reports.rs"]
mod reports;
#[path = "../../../src/api/streaming/rtc/traffic.rs"]
mod traffic;
#[cfg(test)]
mod webrtc;
// Host harness does not initiate external ICE discovery; all production methods
// still type-check, while the UDP tests exercise local sockets only.
#[allow(dead_code)]
#[path = "../../../src/api/streaming/rtc/ice.rs"]
mod ice;
#[allow(dead_code)]
#[path = "../../../src/api/streaming/rtc/transport.rs"]
mod transport;
#[cfg(test)]
mod tests {
    const INPUT_OUTSTANDING_LIMIT: usize = 256; // RX Test 33 baseline policy
    use bytes::Bytes;
    use sctp::{
        Association, AssociationHandle, ClientConfig, DatagramEvent, Endpoint, EndpointConfig,
        Payload, PayloadProtocolIdentifier, ReliabilityType, ServerConfig, TransportConfig,
    };
    use shared::TransportProtocol;
    use std::{
        collections::VecDeque,
        net::SocketAddr,
        sync::Arc,
        time::{Duration, Instant},
    };

    struct Side {
        endpoint: Endpoint,
        address: SocketAddr,
        association: Option<(AssociationHandle, Association)>,
        incoming: VecDeque<(Instant, Bytes)>,
        received: Vec<(u64, u64)>,
        delay: Duration,
        transmitted: usize,
        data_transmits: usize,
        drop_data: usize,
        forward_tsns: usize,
        unordered_forward_entries: usize,
        sacks: usize,
        last_forward: Option<(u32, Instant)>,
        duplicate_forward_under_rtt: usize,
    }

    impl Side {
        fn new(port: u16, server: bool) -> Self {
            let address = SocketAddr::from(([127, 0, 0, 1], port));
            Self {
                endpoint: Endpoint::new(
                    address,
                    TransportProtocol::UDP,
                    Arc::new(EndpointConfig::default()),
                    server.then(|| Arc::new(ServerConfig::default())),
                ),
                address,
                association: None,
                incoming: VecDeque::new(),
                received: Vec::new(),
                delay: Duration::from_millis(5),
                transmitted: 0,
                data_transmits: 0,
                drop_data: 0,
                forward_tsns: 0,
                unordered_forward_entries: 0,
                sacks: 0,
                last_forward: None,
                duplicate_forward_under_rtt: 0,
            }
        }

        fn drive(&mut self, remote: &mut Self, now: Instant, origin: Instant, lose: bool) {
            while self.incoming.front().is_some_and(|(at, _)| *at <= now) {
                let (at, bytes) = self.incoming.pop_front().unwrap();
                if let Some((handle, event)) = self.endpoint.handle(at, remote.address, None, bytes)
                {
                    match event {
                        DatagramEvent::NewAssociation(conn) => {
                            self.association = Some((handle, conn))
                        }
                        DatagramEvent::AssociationEvent(event) => {
                            self.association.as_mut().unwrap().1.handle_event(event)
                        }
                    }
                }
            }
            let Some((_, conn)) = self.association.as_mut() else {
                return;
            };
            if conn.poll_timeout().is_some_and(|at| at <= now) {
                conn.handle_timeout(now);
            }
            while conn.poll().is_some() {}
            for id in conn.stream_ids() {
                let mut stream = conn.stream(id).unwrap();
                while let Some(chunks) = stream.read_sctp().unwrap() {
                    let mut buf = vec![0; chunks.len()];
                    chunks.read(&mut buf).unwrap();
                    if buf.len() >= 8 {
                        self.received.push((
                            u64::from_le_bytes(buf[..8].try_into().unwrap()),
                            now.duration_since(origin).as_millis() as u64,
                        ));
                    }
                }
            }
            while let Some(tx) = conn.poll_transmit(now) {
                if let Payload::RawEncode(packets) = tx.message
                {
                        for packet in packets {
                            self.transmitted += 1;
                            let data = packet.get(12) == Some(&0);
                            self.data_transmits += usize::from(data);
                            if data && self.drop_data > 0 { self.drop_data -= 1; continue; }
                            if lose { continue; }
                            self.forward_tsns += usize::from(packet.get(12) == Some(&192));
                            self.sacks += usize::from(packet.get(12) == Some(&3));
                            if packet.get(12) == Some(&192) {
                                let length = u16::from_be_bytes(packet[14..16].try_into().unwrap()) as usize;
                                for entry in packet[20..12+length].as_chunks::<4>().0 {
                                    if u16::from_be_bytes(entry[..2].try_into().unwrap()) == 2 {
                                        self.unordered_forward_entries += 1;
                                    }
                                }
                                let tsn = u32::from_be_bytes(packet[16..20].try_into().unwrap());
                                if self.last_forward.is_some_and(|(old,at)| old==tsn && now.duration_since(at) < Duration::from_millis(35)) {
                                    self.duplicate_forward_under_rtt += 1;
                                }
                                self.last_forward=Some((tsn, now));
                            }
                            remote
                                .incoming
                                .push_back((now + self.delay, packet));
                        }
                }
            }
        }
    }

    #[test]
    fn control_traffic_remains_proportional_to_reports_at_measured_home_rtt() {
        verify_control_traffic(120_000);
    }

    #[test]
    fn control_traffic_thirty_virtual_minutes_with_repeated_rtt_stalls() {
        verify_control_traffic(1_800_000);
    }

    #[test]
    fn lossless_unreliable_reports_do_not_abandon_unacknowledged_first_transmissions() {
        verify_control_traffic(5_000);
    }

    fn verify_control_traffic(end_ms: u64) {
        let mut client = Side::new(41000, false);
        let mut server = Side::new(41001, true);
        client.delay = Duration::from_millis(35);
        server.delay = Duration::from_millis(35);
        client.association = Some(client.endpoint.connect(
            ClientConfig::new(TransportConfig::default()), server.address).unwrap());
        let start = Instant::now();
        let mut reports = 0;
        let mut windows = Vec::new();
        let mut previous = 0;
        for ms in 0..end_ms+1000 {
            let now = start + Duration::from_millis(ms);
            // Transient delay step, then a healthy 70ms RTT again. FIFO packets
            // retain their scheduled delivery; no payload is fabricated.
            if ms >= 10_000 && ms % 10_000 == 0 { client.delay = Duration::from_millis(350); server.delay = Duration::from_millis(350); }
            if ms >= 10_000 && ms % 10_000 == 1000 { client.delay = Duration::from_millis(35); server.delay = Duration::from_millis(35); }
            if ms == 1000 {
                client.association.as_mut().unwrap().1.open_stream(2, PayloadProtocolIdentifier::Binary)
                    .unwrap().set_reliability_params(true, ReliabilityType::Rexmit, 0).unwrap();
            }
            if (1000..end_ms).contains(&ms) && ms % 8 == 0 {
                let conn = &mut client.association.as_mut().unwrap().1;
                if conn.immediate_send_capacity() >= 43 {
                    let mut bytes = vec![0;43];
                    bytes[..8].copy_from_slice(&ms.to_le_bytes());
                    conn.stream(2).unwrap().write_sctp(&Bytes::from(bytes), PayloadProtocolIdentifier::Binary).unwrap();
                    reports += 1;
                }
            }
            client.drive(&mut server, now, start, false);
            server.drive(&mut client, now, start, false);
            if ms % 1000 == 999 {
                windows.push(client.transmitted - previous);
                previous = client.transmitted;
            }
        }
        eprintln!("70ms RTT duration={end_ms}ms reports={reports} sent={} forward={} sacks={} maxWindow={} tailWindow={}",
            client.transmitted,client.forward_tsns,server.sacks, windows.iter().max().unwrap(), windows.last().unwrap());
        if end_ms < 10_000 {
            assert_eq!(client.forward_tsns, 0, "a healthy first transmission must await its ACK, not be treated as a failed retransmission");
        }
        assert_eq!(client.unordered_forward_entries, 0, "RFC3758 C4 forbids unordered SSN entries");
        eprintln!("duplicate_forward_under_half_rtt={}", client.duplicate_forward_under_rtt);
        assert_eq!(client.duplicate_forward_under_rtt, 0, "unchanged forward progress must not be amplified by ACKs faster than the path RTT");
        assert!(client.transmitted < reports * 3, "control chatter grows without new useful work");
        assert!(*windows.last().unwrap() < 30, "control traffic must settle after reports stop");
    }

    #[test]
    fn retry_limits_count_retries_not_the_initial_transmission() {
        for (retries, losses, attempts, delivered) in [(0, 1, 1, 0), (1, 1, 2, 1), (1, 2, 2, 0)] {
            let mut client = Side::new(43000, false);
            let mut server = Side::new(43001, true);
            client.association = Some(client.endpoint.connect(ClientConfig::new(TransportConfig::default()), server.address).unwrap());
            let start = Instant::now();
            for ms in 0..15_000u64 {
                let now = start + Duration::from_millis(ms);
                if ms == 1000 {
                    client.drop_data = losses;
                    let conn = &mut client.association.as_mut().unwrap().1;
                    conn.open_stream(2, PayloadProtocolIdentifier::Binary).unwrap()
                        .set_reliability_params(true, ReliabilityType::Rexmit, retries).unwrap();
                    conn.stream(2).unwrap().write_sctp(&Bytes::copy_from_slice(&ms.to_le_bytes()), PayloadProtocolIdentifier::Binary).unwrap();
                }
                client.drive(&mut server, now, start, false);
                server.drive(&mut client, now, start, false);
            }
            assert_eq!(client.data_transmits, attempts, "retry limit {retries}, losses {losses}");
            assert_eq!(server.received.len(), delivered);
            if delivered == 0 { assert!(client.forward_tsns > 0, "loss must advance through FORWARD-TSN"); }
        }
    }

    #[derive(Clone, Copy)]
    enum Admission {
        Unbounded,
        Test33AckCap,
        Test34Capacity,
    }

    fn simulate(stall: bool, admission: Admission, drop_forward: bool) -> (u64, u64, usize) {
        simulate_for(stall, admission, drop_forward, 120_000)
    }

    fn simulate_for(
        stall: bool,
        admission: Admission,
        drop_forward: bool,
        duration_ms: u64,
    ) -> (u64, u64, usize) {
        let mut client = Side::new(41000, false);
        let mut server = Side::new(41001, true);
        client.association = Some(
            client
                .endpoint
                .connect(
                    ClientConfig::new(TransportConfig::default()),
                    server.address,
                )
                .unwrap(),
        );
        let start = Instant::now();
        let mut max_buffer = 0;
        // 120 reports/s approximates separate controller and presentation reports.
        // Stop acknowledgements for two seconds, then allow normal delivery.
        for ms in 0..duration_ms {
            let now = start + Duration::from_millis(ms);
            if ms == 1000 {
                let conn = &mut client.association.as_mut().unwrap().1;
                assert!(!conn.is_handshaking());
                conn.open_stream(2, PayloadProtocolIdentifier::Binary)
                    .unwrap()
                    .set_reliability_params(true, ReliabilityType::Rexmit, 0)
                    .unwrap();
            }
            if (1000..duration_ms - 1000).contains(&ms) && ms % 8 == 0 {
                let capacity = client
                    .association
                    .as_ref()
                    .unwrap()
                    .1
                    .immediate_send_capacity();
                let mut stream = client.association.as_mut().unwrap().1.stream(2).unwrap();
                let mut bytes = vec![0; 43];
                bytes[..8].copy_from_slice(&ms.to_le_bytes());
                let admitted = match admission {
                    Admission::Unbounded => true,
                    Admission::Test33AckCap => {
                        stream.buffered_amount().unwrap() + bytes.len() <= INPUT_OUTSTANDING_LIMIT
                    }
                    Admission::Test34Capacity => bytes.len() <= capacity,
                };
                if admitted {
                    stream
                        .write_sctp(&Bytes::from(bytes), PayloadProtocolIdentifier::Binary)
                        .unwrap();
                }
                max_buffer = max_buffer.max(stream.buffered_amount().unwrap());
            }
            let outage = stall && ms >= 40_000 && ms % 40_000 < 2000;
            client.drive(&mut server, now, start, outage && drop_forward);
            server.drive(&mut client, now, start, outage);
        }
        let max_age = server
            .received
            .iter()
            .map(|(sent, received)| received - sent)
            .max()
            .unwrap();
        let tail_age = server
            .received
            .iter()
            .filter(|(sent, _)| *sent >= duration_ms - 10_000)
            .map(|(sent, received)| received - sent)
            .max()
            .unwrap();
        (max_age, tail_age, max_buffer)
    }

    #[test]
    fn thirty_virtual_minutes_of_repeated_outages_do_not_accumulate_input_age() {
        let (age, tail, outstanding) =
            simulate_for(true, Admission::Test34Capacity, true, 1_800_000);
        eprintln!(
            "1800s virtual SCTP, 44 two-second outages: max={age}ms tail={tail}ms outstanding={outstanding}B"
        );
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
        assert!(
            max_buffer > 10_000,
            "baseline must reproduce the hidden backlog"
        );
        assert!(
            tail_age < 50,
            "transport did not recover after acknowledgements resumed"
        );
    }

    #[test]
    fn bounded_reports_remain_fresh_after_repeated_bidirectional_outages() {
        let (max_age, tail_age, max_buffer) = simulate(true, Admission::Test33AckCap, true);
        eprintln!(
            "bidirectional outages: max age={max_age}ms tail={tail_age}ms buffered={max_buffer}B"
        );
        assert!(max_age < 50);
        assert!(tail_age < 50);
        assert!(max_buffer <= INPUT_OUTSTANDING_LIMIT);
    }

    #[test]
    fn test34_capacity_admission_does_not_replay_old_reports_after_ack_outages() {
        let (max_age, tail_age, max_buffer) = simulate(true, Admission::Test34Capacity, false);
        eprintln!(
            "Test34 ACK outages: max age={max_age}ms tail={tail_age}ms outstanding={max_buffer}B"
        );
        assert!(max_age < 50);
        assert!(tail_age < 50);
    }

    #[test]
    fn test34_capacity_admission_recovers_after_bidirectional_outages() {
        let (max_age, tail_age, max_buffer) = simulate(true, Admission::Test34Capacity, true);
        eprintln!(
            "Test34 bidirectional outages: max age={max_age}ms tail={tail_age}ms outstanding={max_buffer}B"
        );
        assert!(max_age < 50);
        assert!(tail_age < 50);
    }
}

// Compile the production mic uplink against the real RTC dependency.
pub(crate) use feedback::bandwidth;
mod api {
    pub mod streaming {
        pub mod rtc {
            pub(crate) use crate::bandwidth;
            pub mod peer {
                pub type RTCPeerConnection = ::rtc::peer_connection::RTCPeerConnection<
                    crate::arrival_feedback::ReceiveFeedback,
                >;
            }
        }
    }
}
#[path = "../../../src/streaming/microphone_capture.rs"]
mod mic_capture;
#[path = "../../../src/streaming/microphone.rs"]
mod mic_state;
mod streaming {
    pub(crate) use super::mic_state as microphone;
    // Hardware input is injected in the peer test. Production capture still
    // compiles above; the real codec has a separate native Opus roundtrip test.
    pub mod microphone_capture {
        pub struct MicrophoneCapture;
        impl MicrophoneCapture {
            pub fn spawn(_: super::microphone::Microphone) -> anyhow::Result<Self> {
                Ok(Self)
            }
        }
    }
}
use mic_state as microphone;
#[path = "../../../src/api_xbox/streaming/rtc/microphone.rs"]
mod mic_uplink;

#[test]
fn real_host_capture_worker_drop_revokes_capture_and_joins_repeatedly() {
    use std::time::{Duration, Instant};
    for _ in 0..20 {
        let mic = mic_state::Microphone::default();
        mic.set_ready(true);
        mic.set_negotiated(true);
        let worker = mic_capture::MicrophoneCapture::spawn(mic.clone()).unwrap();
        assert!(mic.set_on(true));
        let deadline = Instant::now() + Duration::from_secs(2);
        // Exercise the real non-Vita capture error branch and Drop/join, not
        // the uplink stub. Native port teardown still requires device testing.
        while !mic.status().contains("Vita audio input required") {
            assert!(Instant::now() < deadline, "capture worker did not report failure");
            std::thread::yield_now();
        }
        drop(worker);
        assert!(!mic.available());
        assert!(!mic.is_on());
        assert!(mic.begin_capture().is_none());
    }
}
