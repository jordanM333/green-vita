use super::ice;
use crate::api::streaming::rtc::peer::RTCPeerConnection;
use anyhow::{Context, Result};
use bytes::BytesMut;
use rtc::sansio::Protocol;
use rtc::shared::{TaggedBytesMut, TransportContext, TransportProtocol};
use socket2::SockRef;
use std::mem::MaybeUninit;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

// Deliver decrypted media to the application between short socket-drain passes.
// This bounds pre-AU work, not the socket buffer; unread datagrams remain queued.
const RECEIVE_PASS_PACKETS: usize = 32;
const RECEIVE_PASS_TIME: Duration = Duration::from_millis(2);

/// A real empty-socket observation bounds the residence of subsequently received
/// datagrams. Readiness-cache WouldBlock alone is NOT an empty-socket observation.
/// This is an upper bound, not a kernel arrival timestamp or a queue-depth estimate.
#[derive(Default)]
struct ReceiveBoundary {
    last_empty: Option<Instant>,
    last_pass_end: Option<Instant>,
    empty: u64,
    readiness_gap: u64,
    errors: u64,
    rtc_errors: u64,
    last_error: Option<i32>,
    unknown: u64,
    bounded: u64,
    bound_max_us: u128,
    idle_max_us: u128,
    io_call_max_us: u128,
}

impl ReceiveBoundary {
    fn packet(&mut self, dequeued: Instant) {
        if let Some(empty) = self.last_empty {
            self.bounded += 1;
            self.bound_max_us = self
                .bound_max_us
                .max(dequeued.saturating_duration_since(empty).as_micros());
        } else {
            self.unknown += 1;
        }
    }

    fn confirm_empty(&mut self, socket: &UdpSocket) {
        // Do not replace Tokio/Mio's recv_from with a raw receive: on Vita's
        // poll selector, Mio must re-arm its interest after a real WouldBlock.
        // A one-byte, non-consuming peek checks the kernel independently *after*
        // the normal receive path. It cannot consume or reorder a datagram.
        let mut byte = [MaybeUninit::uninit(); 1];
        let started = Instant::now();
        let result = SockRef::from(socket).peek_from(&mut byte);
        let finished = Instant::now();
        match result {
            Ok(_) => {
                self.readiness_gap += 1; // May be a concurrent arrival; not proof of a reactor bug.
                crate::diagnostic::packet(
                    "socket_ready",
                    Default::default(),
                    finished,
                    Some(started),
                    None,
                    0,
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                self.last_empty = Some(started); // Before syscall, so the bound is conservative.
                self.empty += 1;
                crate::diagnostic::packet(
                    "socket_empty",
                    Default::default(),
                    finished,
                    Some(started),
                    None,
                    0,
                );
            }
            Err(error) => {
                self.errors += 1;
                self.last_error = error.raw_os_error();
                crate::diagnostic::packet(
                    "socket_error",
                    Default::default(),
                    finished,
                    Some(started),
                    None,
                    error.raw_os_error().unwrap_or(-1) as u64,
                );
            }
        }
        self.io_call_max_us = self
            .io_call_max_us
            .max(finished.duration_since(started).as_micros());
    }

    fn take_summary(&mut self, now: Instant, rcvbuf: Option<usize>) -> String {
        let empty_ago = self
            .last_empty
            .map(|at| now.saturating_duration_since(at).as_micros().to_string())
            .unwrap_or_else(|| "?".into());
        let summary = format!(
            "Socket empty:{} readyGap:{} err:{}/{} rtcErr:{} rcvbuf:{:?}B lastEmpty:{}us boundMax:{}us unknown:{} boundedPk:{} passGapMax:{}us ioCallMax:{}us",
            self.empty,
            self.readiness_gap,
            self.errors,
            self.last_error.unwrap_or(0),
            self.rtc_errors,
            rcvbuf,
            empty_ago,
            self.bound_max_us,
            self.unknown,
            self.bounded,
            self.idle_max_us,
            self.io_call_max_us,
        );
        *self = Self {
            last_empty: self.last_empty,
            last_pass_end: self.last_pass_end,
            ..Self::default()
        };
        summary
    }
}

/// UDP/ICE transport shared by WebRTC streaming providers.
pub(crate) struct RtcTransport {
    pub(crate) socket: UdpSocket,
    local_addr: SocketAddr,
    recv_buf: Vec<u8>,
    receive_passes: u64,
    receive_budget_hits: u64,
    receive_packets_max: usize,
    receive_pass_max_us: u128,
    receive_rate: super::feedback::ReceiveRate,
    rr_sent: u64,
    feedback_sent: u64,
    send_errors: u64,
    twcc_sent: u64,
    traffic: super::traffic::Traffic,
    pending_write: Option<TaggedBytesMut>,
    tx_budget_hits: u64,
    tx_would_block: u64,
    boundary: ReceiveBoundary,
    receive_buffer_bytes: Option<usize>,
}

impl RtcTransport {
    pub(crate) fn twcc_sent(&self) -> u64 {
        self.twcc_sent
    }
    pub(crate) async fn bind(
        peer: &mut RTCPeerConnection,
        stun_server: &str,
        route_probe: &str,
    ) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .await
            .context("failed to bind UDP socket for WebRTC transport")?;
        let socket_addr = socket
            .local_addr()
            .context("failed to read local UDP socket address")?;
        let local_addr = ice::add_host_candidate(peer, socket_addr.port(), route_probe)
            .context("failed to add local host ICE candidate")?;

        match ice::discover_server_reflexive_candidate(&socket, local_addr, stun_server).await {
            Ok(Some(public_addr)) => {
                if let Err(_error) =
                    ice::add_srflx_candidate(peer, public_addr, local_addr, stun_server)
                {
                    eprintln!("Failed to add server-reflexive ICE candidate");
                }
            }
            Ok(None) => eprintln!("STUN request produced no usable response"),
            Err(error) => eprintln!("STUN discovery failed: {error:#}"),
        }

        ice::local_candidate(peer, String::new(), None)
            .context("failed to signal end-of-candidates")?;

        let receive_buffer_bytes = SockRef::from(&socket).recv_buffer_size().ok();
        Ok(Self {
            socket,
            local_addr,
            recv_buf: vec![0u8; 2048],
            receive_passes: 0,
            receive_budget_hits: 0,
            receive_packets_max: 0,
            receive_pass_max_us: 0,
            receive_rate: super::feedback::ReceiveRate::new(),
            rr_sent: 0,
            feedback_sent: 0,
            send_errors: 0,
            twcc_sent: 0,
            traffic: super::traffic::Traffic::new(),
            pending_write: None,
            tx_budget_hits: 0,
            tx_would_block: 0,
            boundary: ReceiveBoundary::default(),
            receive_buffer_bytes,
        })
    }

    pub(crate) async fn flush(&mut self, peer: &mut RTCPeerConnection) {
        let started = Instant::now();
        let mut sent = 0;
        while let Some(outgoing) = self.pending_write.take().or_else(|| peer.poll_write()) {
            match self
                .socket
                .try_send_to(&outgoing.message, outgoing.transport.peer_addr)
            {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    // Exactly one owned datagram waits; no protocol output is lost
                    // and receive/input service does not await socket writability.
                    self.pending_write = Some(outgoing);
                    self.tx_would_block += 1;
                    break;
                }
                Err(_) => {
                    self.send_errors += 1;
                }
                Ok(_) => {
                    self.traffic.sent(&outgoing.message);
                    if outgoing.message.len() >= 8 && outgoing.message[0] >> 6 == 2 {
                        // SRTCP keeps its first RTCP header clear. Count successful UDP sends,
                        // not server acknowledgements; encrypted REMB contents are not inspected.
                        crate::diagnostic::event(
                            "rtcp_udp_sent_type",
                            0,
                            u64::from(outgoing.message[1]),
                        );
                        match outgoing.message[1] {
                            201 => self.rr_sent += 1,
                            206 => self.feedback_sent += 1,
                            205 if outgoing.message[0] & 0x1f == 15 => self.twcc_sent += 1,
                            _ => {}
                        }
                    }
                }
            }
            sent += 1;
            if sent >= RECEIVE_PASS_PACKETS || started.elapsed() >= RECEIVE_PASS_TIME {
                self.tx_budget_hits += 1;
                break;
            }
        }
    }

    pub(crate) fn receive(&mut self, peer: &mut RTCPeerConnection) {
        let started = Instant::now();
        if let Some(end) = self.boundary.last_pass_end {
            self.boundary.idle_max_us = self
                .boundary
                .idle_max_us
                .max(started.saturating_duration_since(end).as_micros());
        }
        let mut received = 0;
        loop {
            let receive_started = Instant::now();
            let result = self.socket.try_recv_from(&mut self.recv_buf);
            let arrived = Instant::now();
            self.boundary.io_call_max_us = self
                .boundary
                .io_call_max_us
                .max(arrived.duration_since(receive_started).as_micros());
            match result {
                Ok((n, peer_addr)) => {
                    self.boundary.packet(arrived);
                    crate::diagnostic::packet(
                        "udp",
                        crate::diagnostic::udp_identity(&self.recv_buf[..n]),
                        arrived,
                        Some(receive_started),
                        self.boundary.last_empty,
                        n as u64,
                    );
                    self.receive_rate.receive(n, arrived);
                    received += 1;
                    if let Err(_error) = peer.handle_read(TaggedBytesMut {
                        now: arrived,
                        transport: TransportContext {
                            local_addr: self.local_addr,
                            peer_addr,
                            ecn: None,
                            transport_protocol: TransportProtocol::UDP,
                        },
                        message: BytesMut::from(&self.recv_buf[..n]),
                    }) {
                        self.boundary.rtc_errors += 1;
                        eprintln!("Failed to handle WebRTC UDP packet");
                    }
                    self.traffic
                        .received(&self.recv_buf[..n], arrived.elapsed());
                    if received >= RECEIVE_PASS_PACKETS || started.elapsed() >= RECEIVE_PASS_TIME {
                        self.receive_budget_hits += 1;
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    self.boundary.confirm_empty(&self.socket);
                    break;
                }
                Err(error) => {
                    self.boundary.errors += 1;
                    self.boundary.last_error = error.raw_os_error();
                    eprintln!("Failed to receive WebRTC UDP packet: {error}");
                    break;
                }
            }
        }
        self.receive_passes += 1;
        self.receive_packets_max = self.receive_packets_max.max(received);
        self.receive_pass_max_us = self.receive_pass_max_us.max(started.elapsed().as_micros());
        crate::diagnostic::packet(
            "receive_pass",
            Default::default(),
            Instant::now(),
            Some(started),
            self.boundary.last_pass_end,
            received as u64,
        );
        self.boundary.last_pass_end = Some(Instant::now());
    }

    pub(crate) fn take_receive_summary(&mut self) -> String {
        let wire = self.traffic.take_summary(Instant::now());
        let tx_budget = self.tx_budget_hits;
        let tx_blocked = self.tx_would_block;
        let tx_pending = u8::from(self.pending_write.is_some());
        let boundary = self
            .boundary
            .take_summary(Instant::now(), self.receive_buffer_bytes);
        let summary = format!(
            "RX passes:{} budget:{} maxPk:{} max:{}us\nUDP:{} RRsent:{} PSFB:{} txErr:{} TWCCsent:{}\n{wire}\nTX budget:{tx_budget} blocked:{tx_blocked} pending:{tx_pending}\n{boundary}",
            self.receive_passes,
            self.receive_budget_hits,
            self.receive_packets_max,
            self.receive_pass_max_us,
            self.receive_rate.summary(Instant::now()),
            self.rr_sent,
            self.feedback_sent,
            self.send_errors,
            self.twcc_sent,
        );
        self.receive_passes = 0;
        self.receive_budget_hits = 0;
        self.receive_packets_max = 0;
        self.receive_pass_max_us = 0;
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn kernel_peek_distinguishes_pending_datagrams_from_readiness_and_preserves_them() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let remote = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut boundary = ReceiveBoundary::default();
        boundary.packet(Instant::now());
        assert_eq!(boundary.unknown, 1);
        boundary.confirm_empty(&socket);
        let empty = boundary.last_empty.unwrap();
        assert_eq!(boundary.empty, 1);
        remote
            .send_to(b"retained", socket.local_addr().unwrap())
            .unwrap();
        // No reactor turn has occurred. A direct peek sees what a readiness
        // cache may not yet see. It neither consumes bytes nor advances the bound.
        boundary.confirm_empty(&socket);
        assert_eq!(boundary.readiness_gap, 1);
        assert_eq!(boundary.last_empty, Some(empty));
        let mut buf = [0; 32];
        let (n, _) = socket.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"retained");
        boundary.packet(Instant::now());
        assert!(boundary.bound_max_us <= empty.elapsed().as_micros());
        boundary.confirm_empty(&socket);
        assert_eq!(boundary.empty, 2);
        // Include legal zero-byte UDP datagrams: Ok(0) is not an empty socket.
        remote.send_to(&[], socket.local_addr().unwrap()).unwrap();
        boundary.confirm_empty(&socket);
        assert_eq!(boundary.readiness_gap, 2);
        assert_eq!(socket.recv_from(&mut buf).await.unwrap().0, 0);
    }

    #[test]
    fn kernel_bound_includes_scheduler_stalls_and_survives_summary_windows() {
        let start = Instant::now();
        let mut boundary = ReceiveBoundary {
            last_empty: Some(start),
            ..Default::default()
        };
        boundary.packet(start + Duration::from_secs(2));
        assert_eq!(boundary.bound_max_us, 2_000_000);
        let summary = boundary.take_summary(start + Duration::from_secs(2), None);
        assert!(summary.contains("boundMax:2000000us unknown:0"));
        boundary.packet(start + Duration::from_secs(3));
        assert_eq!(boundary.bound_max_us, 3_000_000);
        // A later actual empty observation tightens future bounds, not old ones.
        boundary.last_empty = Some(start + Duration::from_secs(3));
        boundary.take_summary(start + Duration::from_secs(3), None);
        boundary.packet(start + Duration::from_millis(3004));
        assert_eq!(boundary.bound_max_us, 4000);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn udp_receive_pass_is_bounded_and_deferred_send_keeps_its_bytes() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let remote = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let local_addr = socket.local_addr().unwrap();
        let remote_addr = remote.local_addr().unwrap();
        let mut media = rtc::peer_connection::configuration::media_engine::MediaEngine::default();
        let registry = super::super::arrival_feedback::configure(&mut media, vec![]).unwrap();
        let mut peer = rtc::peer_connection::RTCPeerConnectionBuilder::new()
            .with_media_engine(media)
            .with_interceptor_registry(registry)
            .build()
            .unwrap();
        let payload = BytesMut::from(&b"exact deferred datagram"[..]);
        let mut transport = RtcTransport {
            socket,
            local_addr,
            recv_buf: vec![0; 2048],
            receive_passes: 0,
            receive_budget_hits: 0,
            receive_packets_max: 0,
            receive_pass_max_us: 0,
            receive_rate: super::super::feedback::ReceiveRate::new(),
            rr_sent: 0,
            feedback_sent: 0,
            send_errors: 0,
            twcc_sent: 0,
            traffic: super::super::traffic::Traffic::new(),
            tx_budget_hits: 0,
            tx_would_block: 0,
            boundary: ReceiveBoundary::default(),
            receive_buffer_bytes: None,
            pending_write: Some(TaggedBytesMut {
                now: Instant::now(),
                message: payload.clone(),
                transport: TransportContext {
                    local_addr,
                    peer_addr: remote_addr,
                    ecn: None,
                    transport_protocol: TransportProtocol::UDP,
                },
            }),
        };
        transport.socket.writable().await.unwrap();
        transport.flush(&mut peer).await;
        let mut bytes = [0; 2048];
        let (n, _) = tokio::time::timeout(Duration::from_secs(1), remote.recv_from(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&bytes[..n], &payload[..]);
        assert!(transport.pending_write.is_none());
        // The protocol rejects these unnegotiated media datagrams; the test is
        // of socket-pass fairness, not media authentication or SDP negotiation.
        for _ in 0..64 {
            remote.send_to(&[0x80; 12], local_addr).await.unwrap();
        }
        transport.socket.readable().await.unwrap();
        transport.receive(&mut peer);
        assert!(transport.receive_packets_max <= RECEIVE_PASS_PACKETS);
        assert_eq!(transport.receive_budget_hits, 1);
    }
}
