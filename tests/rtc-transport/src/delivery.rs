//! Delivery cadence matters as well as the age of packets that eventually arrive.
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
                        for packet in packets { remote.incoming.push_back((now + Duration::from_millis(50), packet)); }
                    }
                }
            }
        }
    }


fn delivery(immediate: bool, loss: bool) -> (usize, usize, u64, u64) {
    let mut client = Side::new(43000, false);
    let mut server = Side::new(43001, true);
    client.association = Some(client.endpoint.connect(ClientConfig::new(TransportConfig::default()), server.address).unwrap());
    let start = Instant::now();
    let mut feedback_sent = 0;
    for ms in 0..31_000u64 {
        let now = start + Duration::from_millis(ms);
        if ms == 1000 {
            client.association.as_mut().unwrap().1.open_stream(2, PayloadProtocolIdentifier::Binary).unwrap()
                .set_reliability_params(true, ReliabilityType::Rexmit, 0).unwrap();
        }
        // 60 controller and 60 presentation reports/s, sharing the Xbox channel.
        if (1000..30_000).contains(&ms) && ms % 16 == 0 {
            for (size, old_limit) in [(38, 256), (43, 128)] {
                let capacity = client.association.as_ref().unwrap().1.immediate_send_capacity();
                let mut stream = client.association.as_mut().unwrap().1.stream(2).unwrap();
                let mut bytes = vec![0; size];
                bytes[..8].copy_from_slice(&ms.to_le_bytes());
                let accepted = if immediate { size <= capacity }
                    else { stream.buffered_amount().unwrap() + size <= old_limit };
                if accepted {
                    stream.write_sctp(&Bytes::from(bytes), PayloadProtocolIdentifier::Binary).unwrap();
                    if size == 43 { feedback_sent += 1; }
                }
            }
        }
        client.drive(&mut server, now, start, loss && ms > 2000 && ms % 1777 < 25);
        server.drive(&mut client, now, start, loss && ms > 2000 && ms % 1999 < 25);
    }
    let max_age = server.received.iter().map(|(s, r)| r - s).max().unwrap();
    let max_gap = server.received.windows(2).map(|p| p[1].1 - p[0].1).max().unwrap();
    (server.received.len(), feedback_sent, max_age, max_gap)
}

#[test]
fn test33_ack_cap_starves_feedback_on_a_healthy_100ms_round_trip() {
    let old = delivery(false, false);
    let fixed = delivery(true, false);
    eprintln!("100ms RTT (delivered, feedback, age, gap): Test33={old:?} fixed={fixed:?}");
    assert!(old.1 < fixed.1 / 4, "baseline must reproduce feedback starvation");
    assert!(fixed.0 > 3500 && fixed.1 > 1700);
    assert_eq!(fixed.2, 50);
    assert!(fixed.3 <= 16);
}

#[test]
fn fresh_input_and_feedback_continue_through_intermittent_loss() {
    let fixed = delivery(true, true);
    eprintln!("100ms RTT with loss: {fixed:?}");
    assert!(fixed.0 > 3300 && fixed.1 > 1700);
    assert_eq!(fixed.2, 50, "no hidden unsent backlog");
    assert!(fixed.3 <= 64, "brief loss must not cause a seconds-long input pause");
}
