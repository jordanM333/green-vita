//! HA05-HA07: the Xbox sender behaviors returned by HA04-20, HA05-21 and
//! HA06-22, run through production reorder, H.264 assembly, live-edge clock,
//! keyframe-request policy and (HA07) receiver bitrate request, in the HA04
//! `Link` order. The Xbox side is a MODEL calibrated to those captures, not a
//! claim about the console's internals:
//! - 1.1 KB packets: 3 per frame, 5 while a game launches (about 2.6 Mbps
//!   against a 2.1 Mbps sender), 1 after the encoder backed off; or frame
//!   sizes set by scene content alone (HA06-22's encoder sent 2.1-3.7 Mbps of
//!   heavy scenes while the request was 500 kbps);
//! - one FIFO pacer: HA04-20's measured 232 packets/s, 104 packets/s at the
//!   500 kbps floor; or a pacer at about the receiver's REMB (2.04 Mbps at
//!   2000k in HA04-20, 0.93 Mbps at 500k in HA06-22; 3 Mbps is extrapolated)
//!   that speeds up as queued packets near 2 s old on average (libwebrtc's
//!   queue-time limit, which fits HA06-22's send rates);
//! - every keyframe request answered at the next capture by a two-frame
//!   keyframe (21 + 21 packets, 20 + 16 at the floor), as HA04-20 showed
//!   for all 12 requests.
//!
//! The Vita side keeps what the captures showed of AVCDEC and presentation:
//! AUs wait in the worker's queue while AVCDEC takes about 10.5 ms for each
//! (HA06-22 drains), a picture leaves the decoder when three newer AUs have
//! been submitted, and an AU or picture is used only within the local limit
//! since its AU completed: 240 ms through HA06, MAX_LOCAL_CATCH_UP from HA07.
use super::*;
use crate::congestion::ReceiveBudget;
use crate::feedback::VideoCeiling;
use crate::live_edge::LAG_CEILING;
use std::collections::VecDeque;

// One-way network delay; HA04-20 RTT was 6-17 ms.
const NET: Duration = Duration::from_millis(3);
const DECODER_DEPTH: usize = 3;
const VSYNC: Duration = Duration::from_millis(8);
const PACKET_BYTES: usize = 1_100;
const PACKET_BITS: f64 = (PACKET_BYTES * 8) as f64;
/// libwebrtc's pacer queue-time limit, in seconds.
const QUEUE_TIME_LIMIT: f64 = 2.0;
/// FEEDBACK_INTERVAL: how often the receiver sends its request.
const REMB_INTERVAL: Duration = Duration::from_millis(500);
const P_START: &[u8] = &[0x7c, 0x81, 0x88];
const P_MID: &[u8] = &[0x7c, 0x01, 0x99];
const P_END: &[u8] = &[0x7c, 0x41, 0xaa];
const I_START: &[u8] = &[0x7c, 0x85, 0x88];
const I_MID: &[u8] = &[0x7c, 0x05, 0x99];
const I_END: &[u8] = &[0x7c, 0x45, 0xaa];

/// The Vita side: the worker's AU queue, AVCDEC's time per AU, and how long
/// after its AU completed an AU is decoded and its picture shown.
#[derive(Clone, Copy)]
struct Local {
    decode: Duration,
    limit: Duration,
    capacity: usize,
}

/// HA06-22: decode plus output polling took about 10.5 ms per AU in drains.
const AVCDEC: Duration = Duration::from_micros(10_500);
const HA06_LOCAL: Local = Local {
    decode: AVCDEC,
    limit: ms(240),
    capacity: 32,
};
const HA07_LOCAL: Local = Local {
    decode: AVCDEC,
    limit: policy::MAX_LOCAL_CATCH_UP,
    capacity: policy::AU_QUEUE_CAPACITY,
};

/// The receiver bitrate request the pacer follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Remb {
    /// HA06: 2 Mbps; any growth in video's added delay lowers the request.
    Ha06,
    /// HA07: VIDEO_CEILING_BPS; only delay audio shares lowers it.
    Ha07,
}

/// The receiver's request as session.rs computes it, under either policy.
enum Receiver {
    Ha06(ReceiveBudget),
    Ha07(VideoCeiling),
}

impl Receiver {
    fn new(policy: Remb) -> Self {
        match policy {
            Remb::Ha06 => Self::Ha06(ReceiveBudget::new(2_000_000)),
            Remb::Ha07 => Self::Ha07(VideoCeiling::default()),
        }
    }

    fn receive(&mut self, video_ms: u64, audio_ms: u64, at: Instant) {
        match self {
            Self::Ha06(budget) => budget.receive(PACKET_BYTES, video_ms, at),
            Self::Ha07(ceiling) => ceiling.receive(PACKET_BYTES, video_ms, Some(audio_ms), at),
        }
    }

    fn target(&self) -> u32 {
        match self {
            Self::Ha06(budget) => budget.target(),
            Self::Ha07(ceiling) => ceiling.target_bps(),
        }
    }
}

/// The modeled pacer's base rate for a request: calibrated at 2000k
/// (HA04-20) and 500k (HA06-22) only.
fn pacing_bps(remb: u32) -> f64 {
    (f64::from(remb) * 1.02).max(930_000.0)
}

fn p_frame(packets: usize) -> Vec<&'static [u8]> {
    if packets == 1 {
        return vec![P];
    }
    let mut frame = vec![P_START];
    frame.extend(std::iter::repeat_n(P_MID, packets - 2));
    frame.push(P_END);
    frame
}

/// SPS, PPS and an IDR slice in `packets - 2` FU-A fragments.
fn idr_frame(packets: usize) -> Vec<&'static [u8]> {
    let mut frame = vec![SPS, PPS, I_START];
    frame.extend(std::iter::repeat_n(I_MID, packets - 4));
    frame.push(I_END);
    frame
}

const fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

const NEVER: Duration = Duration::from_secs(3_600);

/// When the modeled sender and path change behavior, from the start of a run.
#[derive(Clone, Copy)]
struct Profile {
    /// Encoder overshoot begins (game launch): 5-packet frames.
    launch: Duration,
    /// Encoder backs off to about 0.5 Mbps.
    backoff: Duration,
    /// Pacer slows to the bitrate floor.
    floor: Duration,
    end: Duration,
    /// One scene-change frame of this many packets at this time.
    scene_change: Option<(Duration, usize)>,
    /// Packets put on the wire in this interval are lost (Wi-Fi outage).
    outage: Option<(Duration, Duration)>,
    /// Encoder and pacer stop in this interval; the next capture is a
    /// keyframe. A packet already queued waits (HA05-21: 3.25 s mid-frame).
    pause: Option<(Duration, Duration)>,
    /// One-way path delay by send time. Packets never overtake each other.
    path: fn(Duration) -> Duration,
    /// Scene content in Mbps by capture time. The encoder follows content,
    /// not the request (HA06-22). Otherwise frame sizes follow `launch`.
    content: Option<fn(Duration) -> f64>,
    /// The pacer follows this policy's REMB and the queue-time limit.
    /// Otherwise it runs at HA04-20's measured rates.
    remb: Option<Remb>,
}

fn direct(_: Duration) -> Duration {
    NET
}

const STEADY: Profile = Profile {
    launch: NEVER,
    backoff: NEVER,
    floor: NEVER,
    end: ms(8_000),
    scene_change: None,
    outage: None,
    pause: None,
    path: direct,
    content: None,
    remb: None,
};

/// HA04-20 in sender time: launch 114.6 s, encoder back-off about 118.1 s,
/// pacer at the floor about 119.3 s; two seconds of live video first.
const HA04_20: Profile = Profile {
    launch: ms(2_000),
    backoff: ms(5_500),
    floor: ms(6_700),
    end: ms(20_000),
    ..STEADY
};

impl Profile {
    fn frame_packets(&self, since: Duration) -> usize {
        if since < self.launch {
            3
        } else if since < self.backoff {
            5
        } else {
            1
        }
    }

    fn packets_per_second(&self, since: Duration) -> f64 {
        if since < self.floor { 232.0 } else { 104.0 }
    }

    fn keyframe(&self, since: Duration) -> (usize, usize) {
        if since < self.floor {
            (21, 21)
        } else {
            (20, 16)
        }
    }

    fn lost(&self, since: Duration) -> bool {
        self.outage
            .is_some_and(|(from, to)| (from..to).contains(&since))
    }

    fn paused(&self, since: Duration) -> bool {
        self.pause
            .is_some_and(|(from, to)| (from..to).contains(&since))
    }
}

struct Xbox {
    start: Instant,
    profile: Profile,
    seq: u16,
    next_frame: u32,
    queue: VecDeque<(Instant, rtp::Packet)>,
    credit: f64,
    requests: VecDeque<Instant>,
    follow: Option<usize>,
    keyframes: Vec<Instant>,
    max_backlog: Duration,
    resumed: bool,
    /// Sum of queued packets' capture times (seconds from start), for the
    /// queue-time limit.
    queued_since: f64,
    /// The request the pacer follows, and requests still on the wire.
    estimate: u32,
    rembs: VecDeque<(Instant, u32)>,
    /// Fractional packets carried between frames sized by content.
    carry: f64,
}

impl Xbox {
    fn new(start: Instant, profile: Profile) -> Self {
        Self {
            start,
            profile,
            seq: 1000,
            next_frame: 0,
            queue: VecDeque::new(),
            credit: 0.0,
            requests: VecDeque::new(),
            follow: None,
            keyframes: Vec::new(),
            max_backlog: Duration::ZERO,
            resumed: true,
            queued_since: 0.0,
            estimate: match profile.remb {
                Some(Remb::Ha06) => 2_000_000,
                _ => crate::feedback::VIDEO_CEILING_BPS,
            },
            rembs: VecDeque::new(),
            carry: 0.0,
        }
    }

    fn frame_packets(&mut self, since: Duration) -> usize {
        let Some(content) = self.profile.content else {
            return self.profile.frame_packets(since);
        };
        // Mbps times microseconds per frame: bits per frame.
        self.carry += content(since) * FRAME_US as f64 / PACKET_BITS;
        let packets = self.carry.floor().max(1.0);
        self.carry -= packets;
        packets as usize
    }

    /// Packets per second now: the measured rates, or the request's pacing
    /// rate raised so queued packets average no more than 2 s.
    fn pacing(&self, now: Instant) -> f64 {
        let since = now - self.start;
        if self.profile.remb.is_none() {
            return self.profile.packets_per_second(since);
        }
        let base = pacing_bps(self.estimate) / PACKET_BITS;
        if self.queue.is_empty() {
            return base;
        }
        let queued = self.queue.len() as f64;
        let average_wait = since.as_secs_f64() - self.queued_since / queued;
        base.max(queued / (QUEUE_TIME_LIMIT - average_wait).max(0.001))
    }

    fn capture(&self, frame: u32) -> Instant {
        self.start + Duration::from_micros(u64::from(frame) * FRAME_US)
    }

    fn encode(&mut self, frame: u32, at: Instant) {
        let since = at - self.start;
        if self.profile.paused(since) {
            // Nothing is captured; requests made meanwhile are not answered.
            self.requests.clear();
            self.resumed = false;
            return;
        }
        // The first frame, the first after a pause, and the next frame after a
        // request arrives are keyframes; the frame after one is as large again.
        let requested = self.requests.front().is_some_and(|request| *request <= at);
        let payloads = if frame == 0 || requested || !self.resumed {
            while self.requests.front().is_some_and(|request| *request <= at) {
                self.requests.pop_front();
            }
            self.resumed = true;
            let (keyframe, follow) = self.profile.keyframe(since);
            self.follow = Some(follow);
            self.keyframes.push(at);
            idr_frame(keyframe)
        } else if let Some(follow) = self.follow.take() {
            p_frame(follow)
        } else if let Some((when, packets)) = self.profile.scene_change
            && since >= when
            && since < when + Duration::from_micros(FRAME_US)
        {
            p_frame(packets)
        } else {
            let packets = self.frame_packets(since);
            p_frame(packets)
        };
        let count = payloads.len();
        for (index, payload) in payloads.into_iter().enumerate() {
            let packet = rtp::Packet {
                header: rtp::header::Header {
                    sequence_number: self.seq,
                    timestamp: ts(frame),
                    marker: index + 1 == count,
                    ..Default::default()
                },
                payload: Bytes::from_static(payload),
            };
            self.seq = self.seq.wrapping_add(1);
            self.queued_since += since.as_secs_f64();
            self.queue.push_back((at, packet));
        }
    }

    /// One millisecond: capture due frames, then pace packets onto the wire.
    fn step(&mut self, now: Instant) -> Vec<rtp::Packet> {
        while self.capture(self.next_frame) <= now {
            self.encode(self.next_frame, self.capture(self.next_frame));
            self.next_frame += 1;
        }
        let since = now - self.start;
        while self.rembs.front().is_some_and(|(at, _)| *at <= now) {
            self.estimate = self.rembs.pop_front().unwrap().1;
        }
        if self.profile.paused(since) {
            return Vec::new();
        }
        self.credit += self.pacing(now) / 1000.0;
        let mut sent = Vec::new();
        while self.credit >= 1.0 {
            let Some((captured, packet)) = self.queue.pop_front() else {
                break;
            };
            self.queued_since -= (captured - self.start).as_secs_f64();
            self.max_backlog = self.max_backlog.max(now - captured);
            self.credit -= 1.0;
            sent.push(packet);
        }
        if self.queue.is_empty() {
            self.credit = self.credit.min(1.0);
        }
        sent
    }
}

#[derive(Debug, Default)]
struct Outcome {
    requests: Vec<Duration>,
    keyframes: Vec<Duration>,
    max_backlog: Duration,
    incidents: u64,
    recovered: u64,
    expired: usize,
    /// AUs that missed the local deadline; each abandons the epoch.
    deadlines: usize,
    /// AUs refused by a full worker queue, and the deepest it got.
    full: usize,
    max_queue: usize,
    /// Requests sent to the pacer: (sent, bps).
    rembs: Vec<(Duration, u32)>,
    /// Every presented picture: (presented, captured).
    shown: Vec<(Duration, Duration)>,
    end: Duration,
}

impl Outcome {
    /// The longest time without a newly presented picture after `from`,
    /// including any freeze still running at the end.
    fn longest_freeze(&self, from: Duration) -> Duration {
        let mut last = from;
        let mut longest = Duration::ZERO;
        for (at, _) in self.shown.iter().filter(|(at, _)| *at >= from) {
            longest = longest.max(*at - last);
            last = *at;
        }
        longest.max(self.end.saturating_sub(last))
    }

    /// Worst capture-to-presentation age after `from`.
    fn worst_age(&self, from: Duration) -> Duration {
        self.shown
            .iter()
            .filter(|(at, _)| *at >= from)
            .map(|(at, captured)| at.saturating_sub(*captured))
            .max()
            .unwrap_or_default()
    }

    /// Capture-to-presentation age of the last picture shown.
    fn final_age(&self) -> Duration {
        self.shown
            .last()
            .map(|(at, captured)| at.saturating_sub(*captured))
            .unwrap_or(Duration::MAX)
    }

    /// First picture shown at or after `from`.
    fn first_shown(&self, from: Duration) -> Option<Duration> {
        self.shown.iter().map(|(at, _)| *at).find(|at| *at >= from)
    }

    /// Median capture-to-presentation age of pictures shown in `range`.
    fn median_age(&self, range: std::ops::Range<Duration>) -> Duration {
        let mut ages: Vec<_> = self
            .shown
            .iter()
            .filter(|(at, _)| range.contains(at))
            .map(|(at, captured)| at.saturating_sub(*captured))
            .collect();
        ages.sort_unstable();
        ages.get(ages.len() / 2).copied().unwrap_or(Duration::MAX)
    }

    /// How often the request went down, and the lowest sent.
    fn remb_cuts(&self) -> (usize, u32) {
        let cuts = self.rembs.windows(2).filter(|w| w[1].1 < w[0].1).count();
        let lowest = self.rembs.iter().map(|(_, bps)| *bps).min().unwrap_or(0);
        (cuts, lowest)
    }

    fn report(&self, name: &str, from: Duration) -> String {
        format!(
            "{name}: longest freeze after {:.2}s {}ms, worst shown age {}ms, final age {}ms, \
             {} requests {:?}, {} keyframes, sender backlog max {}ms, incidents {}, \
             recoveries {}, expired pictures {}, AU deadlines {}, queue full {} (max {}), \
             REMB cuts {} lowest {}k",
            from.as_secs_f64(),
            self.longest_freeze(from).as_millis(),
            self.worst_age(from).as_millis(),
            self.final_age().as_millis(),
            self.requests.len(),
            self.requests
                .iter()
                .map(|at| at.as_millis())
                .collect::<Vec<_>>(),
            self.keyframes.len(),
            self.max_backlog.as_millis(),
            self.incidents,
            self.recovered,
            self.expired,
            self.deadlines,
            self.full,
            self.max_queue,
            self.remb_cuts().0,
            self.remb_cuts().1 / 1000
        )
    }
}

fn simulate(profile: Profile, local: Local) -> Outcome {
    let mut link = Link::new();
    link.worker
        .capacity
        .store(local.capacity, Ordering::Relaxed);
    let start = link.start;
    let mut xbox = Xbox::new(start, profile);
    let mut wire: VecDeque<(Instant, rtp::Packet)> = VecDeque::new();
    let mut forwarded = 0;
    let mut receiver = profile.remb.map(Receiver::new);
    let mut next_remb = start;
    // AVCDEC: (AU complete, RTP timestamp, generation) awaiting output.
    let mut decoding: VecDeque<(Instant, u32, usize)> = VecDeque::new();
    let mut decoder_free = start;
    let mut display: VecDeque<(Instant, u32, Instant)> = VecDeque::new();
    let mut outcome = Outcome {
        end: profile.end,
        ..Default::default()
    };
    let mut established = false;
    for tick in 0..profile.end.as_millis() as u64 {
        let now = start + ms(tick);
        let since = now - start;
        for packet in xbox.step(now) {
            if !profile.lost(since) {
                let behind = wire.back().map_or(now, |(at, _)| *at);
                wire.push_back((behind.max(now + (profile.path)(since)), packet));
            }
        }
        while wire.front().is_some_and(|(at, _)| *at <= now) {
            let (at, packet) = wire.pop_front().unwrap();
            let timestamp = packet.header.timestamp;
            link.receive(packet, at);
            // session.rs: video's added delay at delivery. Audio shares the
            // path, so its added delay is the path's.
            if let Some(receiver) = receiver.as_mut() {
                let video = link
                    .worker
                    .edge
                    .lock()
                    .unwrap()
                    .added_delay_ms(timestamp, at)
                    .unwrap_or(0);
                let audio = (profile.path)(since).saturating_sub(NET).as_millis() as u64;
                receiver.receive(video, audio, at);
            }
        }
        if let Some(receiver) = receiver.as_ref()
            && now >= next_remb
        {
            xbox.rembs
                .push_back((now + (profile.path)(since), receiver.target()));
            outcome.rembs.push((since, receiver.target()));
            next_remb = now + REMB_INTERVAL;
        }
        if tick % 4 == 0 {
            // media.rs drain_decoder, then the session pump.
            if link.video.recover_decoder(&link.worker) {
                link.demand = true;
            }
            link.pump(now);
        }
        while forwarded < link.requests.len() {
            xbox.requests
                .push_back(link.requests[forwarded] + (profile.path)(since));
            forwarded += 1;
        }
        // worker.rs: AVCDEC takes `local.decode` per AU, oldest first.
        while decoder_free <= now {
            let Some((complete, timestamp, generation)) =
                link.worker.queue.lock().unwrap().pop_front()
            else {
                break;
            };
            if generation != link.worker.resyncs.load(Ordering::Relaxed) {
                continue;
            }
            let useful = link.worker.edge.lock().unwrap().useful(timestamp, now);
            if now - complete > local.limit || !useful {
                // Abandon the epoch once and ask for a complete IDR.
                link.worker.resyncs.fetch_add(1, Ordering::Relaxed);
                link.worker.recovery.store(true, Ordering::Relaxed);
                outcome.deadlines += 1;
                continue;
            }
            decoder_free = now + local.decode;
            decoding.push_back((complete, timestamp, generation));
            if decoding.len() > DECODER_DEPTH {
                let (complete, timestamp, picture) = decoding.pop_front().unwrap();
                // Older-generation output is discarded; a stale picture expires.
                if picture != generation {
                    continue;
                }
                if decoder_free - complete > local.limit {
                    outcome.expired += 1;
                } else {
                    display.push_back((decoder_free + VSYNC, timestamp, complete));
                }
            }
        }
        outcome.max_queue = outcome
            .max_queue
            .max(link.worker.queue.lock().unwrap().len());
        while display.front().is_some_and(|(at, _, _)| *at <= now) {
            let (at, timestamp, complete) = display.pop_front().unwrap();
            let mut edge = link.worker.edge.lock().unwrap();
            let current = if established {
                edge.presented(timestamp, at);
                edge.can_present(timestamp, at)
            } else {
                established = edge.establish(timestamp, complete, at);
                established
            };
            if current {
                let captured = Duration::from_micros(u64::from(timestamp / 1500) * FRAME_US);
                outcome.shown.push((at - start, captured));
            }
        }
    }
    let edge = link.worker.edge.lock().unwrap();
    outcome.requests = link.requests.iter().map(|at| *at - start).collect();
    outcome.keyframes = xbox.keyframes.iter().map(|at| *at - start).collect();
    outcome.max_backlog = xbox.max_backlog;
    outcome.incidents = edge.incidents;
    outcome.recovered = edge.recovered;
    outcome.full = link.worker.full.load(Ordering::Relaxed);
    outcome
}

#[test]
fn a_game_launch_backlog_plays_late_and_catches_up_without_freezing() {
    let outcome = simulate(HA04_20, HA07_LOCAL);
    let report = outcome.report("HA04-20 model", HA04_20.launch);
    println!("{report}");
    // HA04 in this model: 18 requests and keyframes into the backlog, which
    // peaked at 2.17 s; no picture for 16.8 s. HA05: one request after the
    // backlog drained, no picture for 4.6 s.
    assert_eq!(
        (outcome.incidents, outcome.requests.len()),
        (0, 0),
        "{report}"
    );
    assert_eq!(outcome.keyframes.len(), 1, "startup only: {report}");
    assert!(outcome.longest_freeze(HA04_20.launch) < ms(120), "{report}");
    // Video lags by the sender's backlog while it lasts, then is current.
    assert!(outcome.worst_age(HA04_20.launch) > ms(900), "{report}");
    assert!(outcome.worst_age(HA04_20.launch) < LAG_CEILING, "{report}");
    assert!(outcome.final_age() < ms(100), "{report}");
    assert_eq!(outcome.expired, 0, "{report}");
}

#[test]
fn a_scene_change_frame_longer_than_the_local_budget_is_shown_not_expired() {
    // HA05-21 at 20.53 s: a 79 KB (66-packet) frame took 310 ms to arrive at
    // about 2.1 Mbps, then heavier frames followed. HA05 expired it by
    // first-packet age (au_deadline_recovery) and froze for 1.6 s.
    let profile = Profile {
        launch: ms(3_017),
        backoff: ms(3_600),
        scene_change: Some((ms(3_000), 66)),
        ..STEADY
    };
    let outcome = simulate(profile, HA07_LOCAL);
    let report = outcome.report("HA05-21 scene change", ms(3_000));
    println!("{report}");
    // At most the pictures AVCDEC held while the big frame arrived expire at
    // output (no recovery); the scene change itself is decoded and shown.
    assert!(outcome.expired <= DECODER_DEPTH, "{report}");
    assert_eq!(
        (outcome.incidents, outcome.requests.len()),
        (0, 0),
        "{report}"
    );
    assert!(outcome.worst_age(ms(3_000)) > ms(300), "{report}");
    assert!(outcome.longest_freeze(ms(3_000)) < ms(400), "{report}");
    assert!(outcome.final_age() < ms(100), "{report}");
}

#[test]
fn a_sender_pause_is_held_then_resumes_from_its_keyframe() {
    // HA05-21 at 9.92 s: the Xbox stopped mid-frame for 3.25 s (sequence
    // numbers continuous, audio unaffected), then resumed with an IDR.
    let profile = Profile {
        pause: Some((ms(3_000), ms(6_250))),
        ..STEADY
    };
    let outcome = simulate(profile, HA07_LOCAL);
    let report = outcome.report("HA05-21 pause", ms(3_000));
    println!("{report}");
    // Silence is held, not an incident, and keyframes are requested on the
    // backoff after SILENCE; the paused sender ignores them.
    assert!(
        outcome
            .requests
            .first()
            .is_some_and(|at| (ms(3_900)..ms(4_300)).contains(at)),
        "{report}"
    );
    // The stale tail of the interrupted frame cannot be shown; the sender's
    // resume keyframe is shown once it and AVCDEC's output depth clear.
    let back = outcome.first_shown(ms(6_250)).expect("never resumed");
    assert!(back < ms(6_250) + ms(300), "{report}");
    assert!(outcome.incidents <= 1, "{report}");
    assert_eq!(outcome.incidents, outcome.recovered, "{report}");
    assert!(outcome.final_age() < ms(100), "{report}");
}

#[test]
fn a_keyframe_slower_than_the_local_budget_recovers_at_the_bitrate_floor() {
    // At the floor a two-frame keyframe takes about 350 ms on the wire, and
    // the frames behind it arrive as late.
    let profile = Profile {
        launch: Duration::ZERO,
        backoff: Duration::ZERO,
        floor: Duration::ZERO,
        outage: Some((ms(3_000), ms(3_600))),
        ..STEADY
    };
    let outcome = simulate(profile, HA07_LOCAL);
    let report = outcome.report("floor keyframe", ms(3_000));
    println!("{report}");
    // The outage loses packets: one incident, recovered by one keyframe that
    // arrives slowly, without a loop.
    assert_eq!((outcome.incidents, outcome.recovered), (1, 1), "{report}");
    assert!(outcome.worst_age(ms(3_600)) > ms(240), "{report}");
    assert!(outcome.longest_freeze(ms(3_000)) < ms(1_600), "{report}");
    assert!(outcome.final_age() < ms(100), "{report}");
}

/// A path that slows while video is still arriving: 300 ms more over 400 ms,
/// then a steady 150 ms more than its best.
fn slowing(since: Duration) -> Duration {
    match since.as_millis() as u64 {
        0..3_000 => NET,
        at @ 3_000..3_400 => NET + ms((at - 3_000) * 3 / 4),
        _ => NET + ms(150),
    }
}

#[test]
fn a_slower_path_plays_on_without_an_incident() {
    let outcome = simulate(
        Profile {
            path: slowing,
            ..STEADY
        },
        HA07_LOCAL,
    );
    let report = outcome.report("slower path", ms(3_000));
    println!("{report}");
    assert_eq!(
        (outcome.incidents, outcome.requests.len()),
        (0, 0),
        "{report}"
    );
    assert!(outcome.longest_freeze(ms(3_000)) < ms(120), "{report}");
    assert!(outcome.final_age() > ms(150), "{report}");
}

/// HA06-22-like gameplay: moderate scenes at 1.4 Mbps, and heavy scenes at
/// 2.8 and 3.4 Mbps (HA06-22's heavy scenes: 2.1-3.7 Mbps).
fn gameplay(since: Duration) -> f64 {
    match since.as_millis() {
        8_000..18_000 => 2.8,
        40_000..46_000 => 3.4,
        _ => 1.4,
    }
}

fn gameplay_under(remb: Remb) -> Profile {
    Profile {
        content: Some(gameplay),
        remb: Some(remb),
        end: ms(60_000),
        ..STEADY
    }
}

#[test]
fn the_ha06_request_leaves_gameplay_behind_after_one_heavy_scene() {
    // HA06-22: video sat about 1.3 s behind for over a minute at 500 kbps
    // while audio stayed current. The request fell on video-only lateness;
    // the Xbox then paced below moderate content and only its queue-time
    // limit kept video moving.
    let outcome = simulate(gameplay_under(Remb::Ha06), HA06_LOCAL);
    let report = outcome.report("HA06 request", ms(2_000));
    println!("{report}");
    let (cuts, lowest) = outcome.remb_cuts();
    assert!(cuts >= 3 && lowest == 500_000, "{report}");
    assert!(
        outcome.median_age(ms(2_000)..ms(8_000)) < ms(100),
        "{report}"
    );
    for moderate in [ms(25_000)..ms(40_000), ms(50_000)..ms(60_000)] {
        assert!(outcome.median_age(moderate) > ms(1_200), "{report}");
    }
    assert_eq!(
        (outcome.incidents, outcome.requests.len()),
        (0, 0),
        "{report}"
    );
}

#[test]
fn the_ha07_request_keeps_gameplay_current_and_catches_up_after_heavier_scenes() {
    // Same content, HA07 policy: sender-side lateness never lowers the
    // request, so the pacer keeps up with moderate and 2.8 Mbps scenes. A
    // scene above the pacing rate still lags while it lasts (an extrapolated
    // 3.06 Mbps at 3 Mbps) and is then caught up.
    let outcome = simulate(gameplay_under(Remb::Ha07), HA07_LOCAL);
    let report = outcome.report("HA07 request", ms(2_000));
    println!("{report}");
    assert_eq!(
        outcome.remb_cuts(),
        (0, crate::feedback::VIDEO_CEILING_BPS),
        "{report}"
    );
    for current in [ms(2_000)..ms(40_000), ms(50_000)..ms(60_000)] {
        assert!(outcome.median_age(current) < ms(100), "{report}");
    }
    assert!(outcome.worst_age(ms(40_000)) > ms(300), "{report}");
    assert!(outcome.worst_age(ms(2_000)) < ms(1_000), "{report}");
    assert!(outcome.longest_freeze(ms(2_000)) < ms(60), "{report}");
    assert_eq!(
        (outcome.deadlines, outcome.expired, outcome.full),
        (0, 0, 0),
        "{report}"
    );
    assert_eq!(
        (outcome.incidents, outcome.requests.len()),
        (0, 0),
        "{report}"
    );
    assert!(outcome.final_age() < ms(100), "{report}");
}

#[test]
fn a_drain_faster_than_avcdec_is_caught_up_locally_instead_of_expired() {
    // After the 3.4 Mbps scene the Xbox sends queued light frames faster than
    // AVCDEC's ~95 AUs per second (HA06-22: up to 2.7 s of video per second).
    // HA06's 240 ms limit expired the waiting work: a freeze, an incident and
    // a keyframe request. HA07 decodes and shows all of it.
    let ha06 = simulate(gameplay_under(Remb::Ha07), HA06_LOCAL);
    let ha07 = simulate(gameplay_under(Remb::Ha07), HA07_LOCAL);
    let report = format!(
        "{}\n{}",
        ha06.report("HA06 local", ms(40_000)),
        ha07.report("HA07 local", ms(40_000))
    );
    println!("{report}");
    assert!(ha06.deadlines + ha06.expired > 0, "{report}");
    assert!(ha06.longest_freeze(ms(40_000)) > ms(200), "{report}");
    assert!(ha06.incidents >= 1, "{report}");
    assert_eq!(
        (ha07.deadlines, ha07.expired, ha07.incidents),
        (0, 0, 0),
        "{report}"
    );
    assert!(ha07.longest_freeze(ms(40_000)) < ms(60), "{report}");
    assert!(
        ha07.median_age(ms(50_000)..ms(60_000)) < ms(100),
        "{report}"
    );
}

/// A Wi-Fi queue that keeps growing: 50 ms more each second for 6 s.
fn congesting(since: Duration) -> Duration {
    match since.as_millis() as u64 {
        0..3_000 => NET,
        at @ 3_000..9_000 => NET + ms((at - 3_000) / 20),
        _ => NET + ms(300),
    }
}

#[test]
fn congestion_on_the_shared_path_still_lowers_the_ha07_request() {
    // HA07 ignores lateness audio does not share, not congestion: a growing
    // path queue delays audio too, and the request still falls.
    let outcome = simulate(
        Profile {
            path: congesting,
            content: Some(|_| 1.4),
            remb: Some(Remb::Ha07),
            end: ms(15_000),
            ..STEADY
        },
        HA07_LOCAL,
    );
    let report = outcome.report("HA07 congested path", ms(3_000));
    println!("{report}");
    let (cuts, lowest) = outcome.remb_cuts();
    assert!(
        cuts >= 1 && lowest < crate::feedback::VIDEO_CEILING_BPS,
        "{report}"
    );
}
