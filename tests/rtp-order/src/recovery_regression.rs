//! HA04 regressions for the HA03-19 blackout mechanism. Production reorder,
//! H.264 assembly, live-edge clock and keyframe-request policy run in the same
//! order as media.rs/session.rs; only the decoder sink and Xbox sender are
//! modeled. Synthetic H.264 bytes: no hardware decode or sender response is claimed.
use super::*;
use crate::live_edge::{REQUEST_CEILING, REQUEST_COOLDOWN, Request, State};
use bytes::Bytes;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use streaming::video::VideoDecodeWorker;

const FRAME_US: u64 = 16_667;
const PUMP: Duration = Duration::from_millis(4);
// libx264 baseline 1280x720 parameter sets (PPS 0 -> SPS 0).
const SPS: &[u8] = &[
    0x67, 0x42, 0xc0, 0x20, 0xda, 0x01, 0x40, 0x16, 0xec, 0x04, 0x40, 0, 0, 3, 0, 0x40, 0, 0, 0x1e,
    0x23, 0xc6, 0x0c, 0xa8,
];
const PPS: &[u8] = &[0x68, 0xce, 0x0f, 0xc8];
// SPS, PPS and one IDR slice (PPS 0) in three FU-A fragments (index 3 = middle).
const IDR: [&[u8]; 5] = [
    SPS,
    PPS,
    &[0x7c, 0x85, 0x88],
    &[0x7c, 0x05, 0x99],
    &[0x7c, 0x45, 0xaa],
];
const P: &[u8] = &[0x61, 0xaa, 0xbb];

fn ts(frame: u32) -> u32 {
    frame.wrapping_mul(1500)
}

fn gaps(times: &[Instant]) -> Vec<Duration> {
    times.windows(2).map(|w| w[1] - w[0]).collect()
}

/// A 60 fps sender plus the client's media.rs/session.rs call order.
struct Link {
    start: Instant,
    seq: u16,
    order: reorder::PacketOrder<rtp::Packet>,
    video: video_rtp::VideoRtp,
    worker: VideoDecodeWorker,
    demand: bool,
    last_sent: Option<Instant>,
    requests: Vec<Instant>,
    stale_quarantines: usize,
    late_ignored: usize,
    sent: Vec<(u32, rtp::Packet)>,
}

impl Link {
    fn new() -> Self {
        let mut order = reorder::PacketOrder::default();
        order.enable_repair(true);
        Self {
            start: Instant::now(),
            seq: 1000,
            order,
            video: video_rtp::VideoRtp::new(1280, 720),
            worker: VideoDecodeWorker::default(),
            demand: false,
            last_sent: None,
            requests: Vec::new(),
            stale_quarantines: 0,
            late_ignored: 0,
            sent: Vec::new(),
        }
    }

    fn at(&self, frame: u32) -> Instant {
        self.start + Duration::from_micros(u64::from(frame) * FRAME_US)
    }

    /// Packetize one frame. `lost` indices consume sequence numbers but never
    /// arrive. Returns the packets that do.
    fn send(&mut self, frame: u32, payloads: &[&'static [u8]], lost: &[usize]) -> Vec<rtp::Packet> {
        let mut delivered = Vec::new();
        for (index, payload) in payloads.iter().enumerate() {
            let packet = rtp::Packet {
                header: rtp::header::Header {
                    sequence_number: self.seq,
                    timestamp: ts(frame),
                    marker: index + 1 == payloads.len(),
                    ..Default::default()
                },
                payload: Bytes::from_static(payload),
            };
            self.seq = self.seq.wrapping_add(1);
            self.sent.push((frame, packet.clone()));
            if !lost.contains(&index) {
                delivered.push(packet);
            }
        }
        delivered
    }

    /// media.rs VideoReceiver::receive for one nonempty packet.
    fn receive(&mut self, packet: rtp::Packet, now: Instant) {
        let timestamp = packet.header.timestamp;
        if self
            .worker
            .observe_media(timestamp, packet.header.sequence_number, now)
        {
            self.order.clear();
            self.video.quarantine(&self.worker);
        }
        if !self.screen(timestamp, now) {
            return;
        }
        if let Some(packet) = self.order.push(packet.header.sequence_number, packet, now) {
            self.ordered(packet, now);
        }
        while let Some(packet) = self.order.pop_ready(now) {
            self.ordered(packet, now);
        }
    }

    /// The live-edge ingress deadline in front of reorder and assembly.
    fn screen(&mut self, timestamp: u32, now: Instant) -> bool {
        if self.worker.media_ingress_useful(timestamp, now) {
            return true;
        }
        if self.video.reject_stale(&self.worker, timestamp) {
            self.stale_quarantines += 1;
        } else {
            self.late_ignored += 1;
        }
        false
    }

    fn ordered(&mut self, packet: rtp::Packet, now: Instant) {
        if self.screen(packet.header.timestamp, now) {
            self.video
                .receive_at(&self.worker, packet, now, &mut self.demand);
        }
    }

    /// session.rs pump: live-edge deadline, gap expiry, one request decision.
    fn pump(&mut self, now: Instant) {
        if self.worker.poll_media(now) {
            self.order.clear();
            self.video.quarantine(&self.worker);
        }
        while let Some(packet) = self.order.pop(now) {
            self.ordered(packet, now);
        }
        let demand = std::mem::take(&mut self.demand);
        let decision =
            self.worker
                .edge
                .lock()
                .unwrap()
                .keyframe_request(demand, self.last_sent, now);
        if let Request::Send(_) = decision {
            self.last_sent = Some(now);
            self.requests.push(now);
        }
    }

    fn pump_until(&mut self, from: Instant, until: Instant) {
        let mut now = from;
        while now < until {
            self.pump(now);
            now += PUMP;
        }
    }

    /// Deliver one frame at its nominal time and pump once. Returns whether a
    /// new AU reached the decoder sink.
    fn deliver(&mut self, frame: u32, payloads: &[&'static [u8]], lost: &[usize]) -> bool {
        let now = self.at(frame);
        let before = self.submitted();
        for packet in self.send(frame, payloads, lost) {
            self.receive(packet, now);
        }
        self.pump(now);
        self.submitted() > before
    }

    /// The surface's confirmation of a rendered current picture.
    fn present(&self, frame: u32) {
        let now = self.at(frame) + Duration::from_millis(8);
        let mut edge = self.worker.edge.lock().unwrap();
        if !edge.establish(ts(frame), now, now) {
            edge.presented(ts(frame), now);
        }
    }

    /// Deliver and, when decoded, present one frame; pump until the next.
    fn frame(&mut self, frame: u32, payloads: &[&'static [u8]], lost: &[usize]) -> bool {
        let decoded = self.deliver(frame, payloads, lost);
        if decoded {
            self.present(frame);
        }
        self.pump_until(self.at(frame) + PUMP, self.at(frame + 1));
        decoded
    }

    fn frames(&mut self, frames: std::ops::Range<u32>) {
        for frame in frames {
            self.frame(frame, &[P], &[]);
        }
    }

    /// Wi-Fi outage: frames are sent (sequence numbers advance) but lost.
    fn outage(&mut self, frames: std::ops::Range<u32>) {
        for frame in frames {
            self.send(frame, &[P], &[0]);
            self.pump_until(self.at(frame), self.at(frame + 1));
        }
    }

    /// Live playback: a complete IDR, then P frames.
    fn live(&mut self, frames: u32) {
        assert!(self.frame(0, &IDR, &[]));
        self.frames(1..frames);
        assert_eq!(self.state(), State::Live);
        assert!(self.requests.is_empty());
    }

    fn submitted(&self) -> usize {
        self.worker.submitted.lock().unwrap().len()
    }

    fn state(&self) -> State {
        self.worker.edge.lock().unwrap().state()
    }

    fn incidents_and_recoveries(&self) -> (u64, u64) {
        let edge = self.worker.edge.lock().unwrap();
        (edge.incidents, edge.recovered)
    }
}

#[test]
fn first_idr_broken_by_further_loss_keeps_requests_going_until_decoding_resumes() {
    let mut link = Link::new();
    link.live(60);
    // A 3 s Wi-Fi outage, as in HA03-19 (105.5 -> 108.3 s).
    link.outage(60..240);
    assert_eq!(link.state(), State::AwaitingKeyframe);
    let outage_requests = link.requests.len();
    // Current P frames return. The next two requested IDRs each lose their
    // middle fragment to further loss: neither may be admitted.
    link.frames(240..260);
    assert!(!link.frame(260, &IDR, &[3]), "broken IDR was admitted");
    link.frames(261..330);
    let second_broken = link.at(330);
    assert!(!link.frame(330, &IDR, &[3]), "broken IDR was admitted");
    link.frames(331..450);
    assert_eq!(link.state(), State::AwaitingKeyframe);
    let before_recovery = link.requests.len();
    let admitted_at = link.at(450);
    assert!(
        link.frame(450, &IDR, &[]),
        "intact current IDR not admitted"
    );
    assert_eq!(
        link.state(),
        State::Live,
        "decoding resumed and was presented"
    );
    // The pre-HA04 policy could send at most 3 requests plus one rearm.
    assert!(before_recovery > 4, "only {before_recovery} requests");
    assert!(
        link.requests[..before_recovery]
            .iter()
            .any(|at| *at > second_broken),
        "requests stopped after the broken IDRs"
    );
    // Never a storm, never a stop: every gap within cooldown..ceiling (plus
    // pump granularity), up to the request that produced the intact IDR.
    let during = &link.requests[..before_recovery];
    let spacing = gaps(during);
    assert!(
        spacing.iter().all(|gap| *gap >= REQUEST_COOLDOWN),
        "{spacing:?}"
    );
    assert!(
        spacing.iter().all(|gap| *gap <= REQUEST_CEILING + PUMP),
        "{spacing:?}"
    );
    assert!(admitted_at - *during.last().unwrap() <= REQUEST_CEILING + PUMP);
    // Broken IDRs never reached the decoder; the intact one cut over once.
    assert_eq!(link.worker.cutovers.load(Ordering::Relaxed), 1);
    assert_eq!(link.incidents_and_recoveries(), (1, 1));
    // Live again: no demand and no further requests.
    link.frames(451..520);
    assert_eq!(link.requests.len(), before_recovery);
    // Decoding resumed, so the backoff reset: a new outage starts at 300 ms.
    link.outage(520..640);
    let next = gaps(&link.requests[before_recovery..]);
    for (gap, expected) in next.iter().zip([300, 600]) {
        let expected = Duration::from_millis(expected);
        assert!(*gap >= expected && *gap < expected + PUMP, "{next:?}");
    }
    println!(
        "HA04 broken-IDR model: {outage_requests} requests during a 3 s outage, {before_recovery} before an intact IDR at 7.5 s (pre-HA04: at most 4), gaps {}..{}ms",
        spacing.iter().min().unwrap().as_millis(),
        spacing.iter().max().unwrap().as_millis()
    );
}

#[test]
fn post_idr_frames_that_only_follow_pre_idr_loss_do_not_reenter_recovery() {
    let mut link = Link::new();
    link.live(60);
    link.outage(60..240);
    link.frames(240..259);
    // Loss immediately before the IDR: frame 259's only packet is lost, so a
    // hole separates pre-IDR media from the IDR in the reorder queue.
    link.frame(259, &[P], &[0]);
    assert!(!link.frame(260, &IDR, &[]), "held behind the hole");
    // Post-IDR frames arrive on time and wait behind the same hole until its
    // 60 ms repair deadline; then the IDR and its dependants are released.
    for frame in 261..264 {
        assert!(!link.frame(frame, &[P], &[]));
    }
    assert_eq!(
        link.state(),
        State::AwaitingPicture,
        "IDR admitted after the hole expired"
    );
    assert_eq!(link.worker.cutovers.load(Ordering::Relaxed), 1);
    let submitted = link.submitted();
    let resyncs = link.worker.resyncs.load(Ordering::Relaxed);
    // Still before its first picture: pre-IDR media now arrives late (NACK
    // retransmissions and AP-buffered outage packets with old timestamps).
    let stale: Vec<_> = link
        .sent
        .iter()
        .filter(|(frame, _)| (100..110).contains(frame) || (230..250).contains(frame))
        .map(|(_, packet)| packet.clone())
        .collect();
    let harmless: Vec<_> = link
        .sent
        .iter()
        .filter(|(frame, _)| (250..260).contains(frame))
        .map(|(_, packet)| packet.clone())
        .collect();
    let now = link.at(264) - Duration::from_millis(1);
    for packet in stale.into_iter().chain(harmless) {
        link.receive(packet, now);
    }
    assert_eq!(link.late_ignored, 30, "stale pre-IDR media not recognized");
    assert_eq!(
        link.stale_quarantines, 0,
        "pre-IDR media re-entered recovery"
    );
    assert_eq!(link.state(), State::AwaitingPicture);
    assert!(!link.video.waiting_for_keyframe());
    assert_eq!(
        link.worker.resyncs.load(Ordering::Relaxed),
        resyncs,
        "the admitted epoch was invalidated"
    );
    // Post-IDR pictures continue the new reference chain and confirm recovery.
    link.frames(264..300);
    assert_eq!(link.submitted(), submitted + 36);
    assert_eq!(link.state(), State::Live);
    assert_eq!(link.incidents_and_recoveries(), (1, 1));
}

#[test]
fn stale_media_beyond_the_admitted_idr_still_quarantines() {
    // The late-packet exemption is narrow: media the assembler has not passed
    // that is itself beyond the recovery budget still breaks the chain.
    let mut link = Link::new();
    link.live(60);
    link.outage(60..240);
    link.frames(240..260);
    assert!(link.deliver(260, &IDR, &[]));
    assert_eq!(link.state(), State::AwaitingPicture);
    let late = link.at(262) + Duration::from_millis(400);
    let packet = link.send(262, &[P], &[]).remove(0);
    link.receive(packet, late);
    assert_eq!(link.stale_quarantines, 1);
    assert_eq!(link.state(), State::AwaitingKeyframe);
    assert!(link.video.waiting_for_keyframe());
    // ... and recovery keeps asking instead of stopping.
    let before = link.requests.len();
    link.pump_until(late, late + Duration::from_secs(5));
    assert!(link.requests.len() >= before + 5);
}
