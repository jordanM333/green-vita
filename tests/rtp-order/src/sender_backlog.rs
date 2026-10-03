//! HA05/HA06: the Xbox sender behaviors returned by HA04-20 and HA05-21, run
//! through production reorder, H.264 assembly, live-edge clock and
//! keyframe-request policy in the HA04 `Link` order. The Xbox side is a MODEL
//! calibrated to those captures, not a claim about the console's internals:
//! - 1.1 KB packets: 3 per frame, 5 while a game launches (about 2.6 Mbps
//!   against a 2.1 Mbps sender), 1 after the encoder backed off;
//! - one FIFO pacer: 232 packets/s, 104 packets/s at the 500 kbps floor;
//! - every keyframe request answered at the next capture by a two-frame
//!   keyframe (21 + 21 packets, 20 + 16 at the floor), as HA04-20 showed
//!   for all 12 requests.
//!
//! The Vita side keeps what the captures showed of AVCDEC and presentation: a
//! picture leaves the decoder when three newer AUs have been submitted, and a
//! picture whose AU completed more than the 240 ms local budget earlier is
//! expired instead of shown.
use super::*;
use crate::live_edge::LAG_CEILING;
use std::collections::VecDeque;

// One-way network delay; HA04-20 RTT was 6-17 ms.
const NET: Duration = Duration::from_millis(3);
const DECODER_DEPTH: usize = 3;
const LOCAL_BUDGET: Duration = Duration::from_millis(240);
const VSYNC: Duration = Duration::from_millis(8);
const P_START: &[u8] = &[0x7c, 0x81, 0x88];
const P_MID: &[u8] = &[0x7c, 0x01, 0x99];
const P_END: &[u8] = &[0x7c, 0x41, 0xaa];
const I_START: &[u8] = &[0x7c, 0x85, 0x88];
const I_MID: &[u8] = &[0x7c, 0x05, 0x99];
const I_END: &[u8] = &[0x7c, 0x45, 0xaa];

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
        }
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
            p_frame(self.profile.frame_packets(since))
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
        if self.profile.paused(since) {
            return Vec::new();
        }
        self.credit += self.profile.packets_per_second(since) / 1000.0;
        let mut sent = Vec::new();
        while self.credit >= 1.0 {
            let Some((captured, packet)) = self.queue.pop_front() else {
                break;
            };
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

    fn report(&self, name: &str, from: Duration) -> String {
        format!(
            "{name}: longest freeze after {:.2}s {}ms, worst shown age {}ms, final age {}ms, \
             {} requests {:?}, {} keyframes, sender backlog max {}ms, incidents {}, \
             recoveries {}, expired pictures {}",
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
            self.expired
        )
    }
}

fn simulate(profile: Profile) -> Outcome {
    let mut link = Link::new();
    let start = link.start;
    let mut xbox = Xbox::new(start, profile);
    let mut wire: VecDeque<(Instant, rtp::Packet)> = VecDeque::new();
    let mut forwarded = 0;
    let mut seen = 0;
    // AVCDEC: (AU complete, RTP timestamp, decode epoch) awaiting output.
    let mut decoding: VecDeque<(Instant, u32, usize)> = VecDeque::new();
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
            link.receive(packet, at);
        }
        if tick % 4 == 0 {
            link.pump(now);
        }
        while forwarded < link.requests.len() {
            xbox.requests
                .push_back(link.requests[forwarded] + (profile.path)(since));
            forwarded += 1;
        }
        let epoch = link.worker.resyncs.load(Ordering::Relaxed);
        let submitted = link.worker.submitted_times.lock().unwrap().clone();
        for (complete, timestamp) in submitted.into_iter().skip(seen) {
            seen += 1;
            decoding.push_back((complete, timestamp, epoch));
            if decoding.len() > DECODER_DEPTH {
                let (complete, timestamp, picture_epoch) = decoding.pop_front().unwrap();
                // Older-epoch output is discarded; a stale picture expires.
                if picture_epoch != epoch {
                    continue;
                }
                if now - complete > LOCAL_BUDGET {
                    outcome.expired += 1;
                } else {
                    display.push_back((now + VSYNC, timestamp, complete));
                }
            }
        }
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
    outcome
}

#[test]
fn a_game_launch_backlog_plays_late_and_catches_up_without_freezing() {
    let outcome = simulate(HA04_20);
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
    let outcome = simulate(profile);
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
    let outcome = simulate(profile);
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
    let outcome = simulate(profile);
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
    let outcome = simulate(Profile {
        path: slowing,
        ..STEADY
    });
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
