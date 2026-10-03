//! HA05: the HA04-20 game-launch freeze. Production reorder, H.264 assembly,
//! live-edge clock and keyframe-request policy run in the HA04 `Link` order.
//! The Xbox side is a MODEL calibrated to the HA04-20 capture, not a claim
//! about the console's encoder or pacer internals:
//! - 1.1 KB packets: 3 per frame, 5 once the game launched (about 2.6 Mbps
//!   against a 2.1 Mbps sender), 1 after the encoder backed off;
//! - one FIFO pacer: 232 packets/s, 104 packets/s at the 500 kbps floor;
//! - every keyframe request answered at the next capture by a two-frame
//!   keyframe (21 + 21 packets, 20 + 16 at the floor), as HA04-20 showed
//!   for all 12 requests.
//!
//! The Vita side keeps what HA04-20 showed of AVCDEC and presentation: a
//! picture leaves the decoder when three newer AUs have been submitted, and a
//! picture older than the 240 ms local budget is expired instead of shown.
use super::*;
use crate::live_edge::{BACKLOG, CATCH_UP, INGRESS_BUDGET, MEDIA_BUDGET};
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

/// When the modeled sender and path change behavior, from the start of a run.
#[derive(Clone, Copy)]
struct Profile {
    /// Encoder overshoot begins (game launch).
    launch: Duration,
    /// Encoder backs off to about 0.5 Mbps.
    backoff: Duration,
    /// Pacer slows to the bitrate floor.
    floor: Duration,
    end: Duration,
    /// Packets put on the wire in this interval are lost (Wi-Fi outage).
    outage: Option<(Duration, Duration)>,
    /// One-way path delay by send time. Packets never overtake each other.
    path: fn(Duration) -> Duration,
}

fn direct(_: Duration) -> Duration {
    NET
}

/// HA04-20 in sender time: launch 114.6 s, encoder back-off about 118.1 s,
/// pacer at the floor about 119.3 s; two seconds of live video first.
const HA04_20: Profile = Profile {
    launch: ms(2_000),
    backoff: ms(5_500),
    floor: ms(6_700),
    end: ms(20_000),
    outage: None,
    path: direct,
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
        }
    }

    fn capture(&self, frame: u32) -> Instant {
        self.start + Duration::from_micros(u64::from(frame) * FRAME_US)
    }

    fn encode(&mut self, frame: u32, at: Instant) {
        let since = at - self.start;
        // The first frame, and the next frame after a request arrives, is a
        // keyframe; the frame after it is as large again.
        let requested = self.requests.front().is_some_and(|request| *request <= at);
        let payloads = if frame == 0 || requested {
            while self.requests.front().is_some_and(|request| *request <= at) {
                self.requests.pop_front();
            }
            let (keyframe, follow) = self.profile.keyframe(since);
            self.follow = Some(follow);
            self.keyframes.push(at);
            idr_frame(keyframe)
        } else if let Some(follow) = self.follow.take() {
            p_frame(follow)
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
        self.credit += self.profile.packets_per_second(now - self.start) / 1000.0;
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
    caught_up: u64,
    catch_up_late: u64,
    /// Every presented picture: (presented, captured).
    shown: Vec<(Duration, Duration)>,
    /// First departure from Live after it was established.
    incident_at: Option<Duration>,
    /// When the sender's backlog first fell below BACKLOG after the incident.
    drained_at: Option<Duration>,
    end: Duration,
}

impl Outcome {
    fn incident(&self) -> Duration {
        self.incident_at.expect("no incident")
    }

    /// The longest time without a newly presented picture after the
    /// incident, including a freeze still running at the end.
    fn longest_freeze(&self) -> Duration {
        let mut last = self.incident();
        let mut longest = Duration::ZERO;
        for (at, _) in self.shown.iter().filter(|(at, _)| *at >= self.incident()) {
            longest = longest.max(*at - last);
            last = *at;
        }
        longest.max(self.end.saturating_sub(last))
    }

    /// When pictures flowed again after the incident: the first of 60 in a
    /// row (one second) with no gap over 50 ms, ending in Live.
    fn resumed(&self) -> Option<Duration> {
        let shown: Vec<_> = self
            .shown
            .iter()
            .map(|(at, _)| *at)
            .filter(|at| *at >= self.incident())
            .collect();
        (0..shown.len().saturating_sub(60)).find_map(|start| {
            shown[start..start + 60]
                .windows(2)
                .all(|pair| pair[1] - pair[0] <= ms(50))
                .then_some(shown[start])
        })
    }

    /// Worst capture-to-presentation age after the incident.
    fn worst_age(&self) -> Duration {
        self.shown
            .iter()
            .filter(|(at, _)| *at >= self.incident())
            .map(|(at, captured)| at.saturating_sub(*captured))
            .max()
            .unwrap_or_default()
    }

    fn report(&self, name: &str) -> String {
        format!(
            "{name}: incident {:.2}s, sender backlog under {}ms at {:?}, resumed {:?}, \
             longest freeze {}ms, {} requests {:?}, {} keyframes, sender backlog max {}ms, \
             incidents {}, recoveries {}, catch-ups ok/late {}/{}, worst shown age {}ms",
            self.incident().as_secs_f64(),
            BACKLOG.as_millis(),
            self.drained_at,
            self.resumed(),
            self.longest_freeze().as_millis(),
            self.requests.len(),
            self.requests
                .iter()
                .map(|at| at.as_millis())
                .collect::<Vec<_>>(),
            self.keyframes.len(),
            self.max_backlog.as_millis(),
            self.incidents,
            self.recovered,
            self.caught_up,
            self.catch_up_late,
            self.worst_age().as_millis()
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
    // AVCDEC: (first packet, RTP timestamp, decode epoch) awaiting output.
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
        for (first_packet, timestamp) in submitted.into_iter().skip(seen) {
            seen += 1;
            decoding.push_back((first_packet, timestamp, epoch));
            if decoding.len() > DECODER_DEPTH {
                let (first_packet, timestamp, picture_epoch) = decoding.pop_front().unwrap();
                // Older-epoch output is discarded; a stale picture expires.
                if picture_epoch == epoch && now - first_packet <= LOCAL_BUDGET {
                    display.push_back((now + VSYNC, timestamp, first_packet));
                }
            }
        }
        while display.front().is_some_and(|(at, _, _)| *at <= now) {
            let (at, timestamp, first_packet) = display.pop_front().unwrap();
            let mut edge = link.worker.edge.lock().unwrap();
            let current = if established {
                edge.presented(timestamp, at);
                edge.can_present(timestamp, at)
            } else {
                established = edge.establish(timestamp, first_packet, at);
                established
            };
            if current {
                let captured = Duration::from_micros(u64::from(timestamp / 1500) * FRAME_US);
                outcome.shown.push((at - start, captured));
            }
        }
        let state = link.worker.edge.lock().unwrap().state();
        if established && outcome.incident_at.is_none() && state != State::Live {
            outcome.incident_at = Some(since);
        }
        let backlogged = xbox
            .queue
            .front()
            .is_some_and(|(captured, _)| now - *captured >= BACKLOG);
        if outcome.incident_at.is_some() && !backlogged && outcome.drained_at.is_none() {
            outcome.drained_at = Some(since);
        }
    }
    let edge = link.worker.edge.lock().unwrap();
    outcome.requests = link.requests.iter().map(|at| *at - start).collect();
    outcome.keyframes = xbox.keyframes.iter().map(|at| *at - start).collect();
    outcome.max_backlog = xbox.max_backlog;
    outcome.incidents = edge.incidents;
    outcome.recovered = edge.recovered;
    outcome.caught_up = edge.caught_up;
    outcome.catch_up_late = edge.catch_up_late;
    outcome
}

#[test]
fn game_launch_backlog_gets_no_keyframe_requests_until_it_drains_then_recovers() {
    let outcome = simulate(HA04_20);
    let report = outcome.report("HA04-20 model");
    println!("{report}");
    // HA04 in the same model: 18 requests and 18 keyframes into the backlog,
    // which peaked at 2.17 s and never drained; no picture for 16.8 s.
    let drained = outcome.drained_at.expect("sender backlog never drained");
    assert!(
        outcome
            .requests
            .iter()
            .all(|at| *at < outcome.incident() || *at >= drained),
        "requested into the backlog: {report}"
    );
    assert_eq!(outcome.requests.len(), 1, "{report}");
    assert_eq!(
        outcome.keyframes.len(),
        2,
        "startup + one recovery: {report}"
    );
    assert!(outcome.max_backlog < ms(1_100), "{report}");
    // One incident and one recovery: the keyframe's own transmission at the
    // floor (about 350 ms) no longer re-triggers recovery.
    assert_eq!((outcome.incidents, outcome.recovered), (1, 1), "{report}");
    assert_eq!(
        (outcome.caught_up, outcome.catch_up_late),
        (1, 0),
        "{report}"
    );
    let resumed = outcome.resumed().expect("video never resumed");
    assert!(resumed <= drained + ms(600), "{report}");
    assert!(outcome.longest_freeze() < ms(5_000), "{report}");
    // The allowance bounds lateness by MEDIA_BUDGET, plus decoder output
    // depth and one refresh.
    assert!(
        outcome.worst_age() <= MEDIA_BUDGET + ms(17 * DECODER_DEPTH as u64) + VSYNC,
        "{report}"
    );
}

#[test]
fn a_keyframe_slower_than_the_budget_recovers_at_the_bitrate_floor() {
    // At the floor a two-frame keyframe takes about 350 ms on the wire, and
    // the frames behind it arrive as late: more than INGRESS_BUDGET.
    let outcome = simulate(Profile {
        launch: Duration::ZERO,
        backoff: Duration::ZERO,
        floor: Duration::ZERO,
        end: ms(8_000),
        outage: Some((ms(3_000), ms(3_600))),
        path: direct,
    });
    let report = outcome.report("floor keyframe");
    println!("{report}");
    assert!(outcome.worst_age() > INGRESS_BUDGET, "{report}");
    assert_eq!((outcome.incidents, outcome.recovered), (1, 1), "{report}");
    assert_eq!(
        (outcome.caught_up, outcome.catch_up_late),
        (1, 0),
        "{report}"
    );
    // Silence keeps the HA04 schedule: the request made during the outage is
    // answered into it, and the next one, after it, recovers.
    let after = outcome
        .requests
        .iter()
        .filter(|at| **at >= ms(3_600))
        .count();
    assert_eq!(after, 1, "{report}");
    let resumed = outcome.resumed().expect("video never resumed");
    assert!(resumed < ms(3_600) + CATCH_UP, "{report}");
}

/// A path that slows while video is still arriving: 300 ms more over 400 ms,
/// then a steady 150 ms more than its best (within INGRESS_BUDGET, above BACKLOG).
fn slowing(since: Duration) -> Duration {
    match since.as_millis() as u64 {
        0..3_000 => NET,
        at @ 3_000..3_400 => NET + ms((at - 3_000) * 3 / 4),
        _ => NET + ms(150),
    }
}

#[test]
fn a_steadily_slower_path_is_not_mistaken_for_a_backlog() {
    let outcome = simulate(Profile {
        launch: ms(60_000),
        backoff: ms(60_000),
        floor: ms(60_000),
        end: ms(8_000),
        outage: None,
        path: slowing,
    });
    let report = outcome.report("slower path");
    println!("{report}");
    // Video kept arriving, more than INGRESS_BUDGET late and then about
    // 150 ms late: nothing is requested until that lateness has persisted
    // for REQUEST_CEILING (from about the incident); then the backoff resumes.
    let first = *outcome
        .requests
        .iter()
        .find(|at| **at >= outcome.incident())
        .expect("never requested after the incident");
    assert!(first >= outcome.incident() + REQUEST_CEILING, "{report}");
    assert!(
        first <= outcome.incident() + REQUEST_CEILING + ms(100),
        "{report}"
    );
    assert_eq!((outcome.incidents, outcome.recovered), (1, 1), "{report}");
    // The keyframe needs a 300 ms round trip on this path.
    let resumed = outcome.resumed().expect("video never resumed");
    assert!(resumed < first + ms(1_000), "{report}");
}
