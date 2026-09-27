use super::ice;
use crate::api::streaming::rtc::peer::RTCPeerConnection;
use anyhow::{Context, Result};
use bytes::BytesMut;
use rtc::sansio::Protocol;
use rtc::shared::{TaggedBytesMut, TransportContext, TransportProtocol};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

// Deliver decrypted media to the application between short socket-drain passes.
// This bounds pre-AU work, not the socket buffer; unread datagrams remain queued.
const RECEIVE_PASS_PACKETS: usize = 32;
const RECEIVE_PASS_TIME: Duration = Duration::from_millis(2);

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
        let mut received = 0;
        loop {
            match self.socket.try_recv_from(&mut self.recv_buf) {
                Ok((n, peer_addr)) => {
                    let arrived = Instant::now();
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
                        eprintln!("Failed to handle WebRTC UDP packet");
                    }
                    self.traffic
                        .received(&self.recv_buf[..n], arrived.elapsed());
                    if received >= RECEIVE_PASS_PACKETS || started.elapsed() >= RECEIVE_PASS_TIME {
                        self.receive_budget_hits += 1;
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    eprintln!("Failed to receive WebRTC UDP packet: {error}");
                    break;
                }
            }
        }
        self.receive_passes += 1;
        self.receive_packets_max = self.receive_packets_max.max(received);
        self.receive_pass_max_us = self.receive_pass_max_us.max(started.elapsed().as_micros());
    }

    pub(crate) fn take_receive_summary(&mut self) -> String {
        let summary = format!(
            "RX passes:{} budget:{} maxPk:{} max:{}us\nUDP:{} RRsent:{} PSFB:{} txErr:{} TWCCsent:{}\n{}",
            self.receive_passes,
            self.receive_budget_hits,
            self.receive_packets_max,
            self.receive_pass_max_us,
            self.receive_rate.summary(Instant::now()),
            self.rr_sent,
            self.feedback_sent,
            self.send_errors,
            self.twcc_sent,
            format!(
                "{}\nTX budget:{} blocked:{} pending:{}",
                self.traffic.take_summary(Instant::now()),
                self.tx_budget_hits,
                self.tx_would_block,
                u8::from(self.pending_write.is_some())
            ),
        );
        self.receive_passes = 0;
        self.receive_budget_hits = 0;
        self.receive_packets_max = 0;
        self.receive_pass_max_us = 0;
        summary
    }
}
