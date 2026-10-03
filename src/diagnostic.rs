//! Diagnostic-only, bounded first-incident capture. No payloads or network arrival claims.
//! Recording uses try_lock and preallocated records; only teardown writes files.
use std::collections::VecDeque;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Mutex,
    atomic::{AtomicU8, AtomicU64, Ordering},
};
use std::time::Instant;

#[path = "display_probe.rs"]
pub(crate) mod display_probe;

pub(crate) const LABEL: &str =
    "HARDWARE ACCEPTANCE CANDIDATE — PHYSICAL VITA ACCEPTANCE PENDING — HA07 LIVE VIDEO UNVERIFIED";
const PRE_US: u64 = 12_000_000;
const POST_US: u64 = 10_000_000;
const LIMIT_US: u64 = 180_000_000;
const INITIAL_US: u64 = 3_000_000;
const CAP: usize = 49_152;
const INITIAL_CAP: usize = 16_384;
const SNAP_CAP: usize = 32;
const SNAP_BYTES: usize = 8192;
pub(crate) const UNKNOWN: u64 = u64::MAX;
static CAPTURE: Mutex<Option<Capture>> = Mutex::new(None);
static STATE: AtomicU8 = AtomicU8::new(0);
static SKIPPED: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Identity {
    pub(crate) ssrc: u32,
    pub(crate) timestamp: u32,
    pub(crate) sequence: u16,
    // 0 = unauthenticated UDP header / unassociated downstream event, 1 = video, 2 = audio.
    pub(crate) media: u8,
    // bit 0: valid RTP header; bit 1: marker; bit 2: nonempty media payload.
    pub(crate) flags: u8,
}

#[derive(Clone, Copy, Debug)]
struct Event {
    at: u64,
    stage: &'static str,
    epoch: u32,
    id: Identity,
    a: u64,
    b: u64,
    c: u64,
}

const _: () = assert!(std::mem::size_of::<Event>() <= 64);

#[derive(Default)]
struct TriggerClock {
    source: Option<(u32, u32)>,
    timestamp: u32,
    ticks: u64,
    start: u64,
    minimum: i128,
    pressure: Option<u64>,
}
impl TriggerClock {
    fn receive(&mut self, e: Event) -> bool {
        if self.source != Some((e.epoch, e.id.ssrc)) {
            *self = Self {
                source: Some((e.epoch, e.id.ssrc)),
                timestamp: e.id.timestamp,
                start: e.at,
                ..Self::default()
            };
            return false;
        }
        let forward = e.id.timestamp.wrapping_sub(self.timestamp);
        if forward == 0 || forward >= 1 << 31 {
            return false;
        }
        if forward > 90_000 * 120 {
            self.source = None;
            return false;
        }
        self.timestamp = e.id.timestamp;
        self.ticks += u64::from(forward);
        let offset = i128::from(e.at.saturating_sub(self.start))
            - i128::from(self.ticks) * 1_000_000 / 90_000;
        self.minimum = self.minimum.min(offset);
        if offset - self.minimum < 250_000 {
            self.pressure = None;
            return false;
        }
        let since = *self.pressure.get_or_insert(e.at);
        e.at.saturating_sub(since) >= 500_000
    }
}

struct Capture {
    start: Instant,
    epoch: u32,
    initial: Vec<Event>,
    pre: VecDeque<Event>,
    post: Vec<Event>,
    snapshots: VecDeque<(u64, String)>,
    initial_snapshots: Vec<(u64, String)>,
    post_snapshots: Vec<(u64, String)>,
    first_video: Option<u64>,
    trigger: Option<u64>,
    frozen: Option<u64>,
    reason: &'static str,
    truncated: u64,
    clock: TriggerClock,
}
impl Capture {
    fn new(start: Instant) -> Self {
        Self {
            start,
            epoch: 0,
            initial: Vec::with_capacity(INITIAL_CAP),
            pre: VecDeque::with_capacity(CAP),
            post: Vec::with_capacity(CAP),
            snapshots: VecDeque::with_capacity(SNAP_CAP),
            initial_snapshots: Vec::with_capacity(4),
            post_snapshots: Vec::with_capacity(12),
            first_video: None,
            trigger: None,
            frozen: None,
            reason: "recording",
            truncated: 0,
            clock: TriggerClock::default(),
        }
    }
    fn us(&self, at: Instant) -> u64 {
        at.saturating_duration_since(self.start).as_micros() as u64
    }
    fn tick(&mut self, at: u64) {
        if self.frozen.is_some() {
            return;
        }
        if self
            .trigger
            .is_some_and(|t| at.saturating_sub(t) >= POST_US)
        {
            self.frozen = Some(at);
            self.reason = "INCIDENT CAPTURED";
        } else if self.trigger.is_none()
            && self
                .first_video
                .is_some_and(|t| at.saturating_sub(t) >= LIMIT_US)
        {
            self.frozen = Some(at);
            self.reason = "NOT REPRODUCED DURING CAPTURE";
        }
    }
    fn push(&mut self, e: Event) {
        self.tick(e.at);
        if self.frozen.is_some() {
            return;
        }
        if e.stage == "rtc" && e.id.media == 1 && e.id.flags & 4 != 0 {
            self.first_video.get_or_insert(e.at);
            if self.trigger.is_none() && self.clock.receive(e) {
                self.trigger = Some(e.at);
            }
        }
        if self
            .first_video
            .is_none_or(|t| e.at.saturating_sub(t) <= INITIAL_US)
        {
            if self.initial.len() < INITIAL_CAP {
                self.initial.push(e);
            } else {
                self.truncated += 1;
            }
        }
        if self.trigger.is_some() {
            if self.post.len() < CAP {
                self.post.push(e);
            } else {
                self.truncated += 1;
                self.frozen = Some(e.at);
                self.reason = "CAPTURE INCOMPLETE: capacity";
            }
        } else {
            while self
                .pre
                .front()
                .is_some_and(|old| e.at.saturating_sub(old.at) > PRE_US)
            {
                self.pre.pop_front();
            }
            if self.pre.len() == CAP {
                self.pre.pop_front();
                self.truncated += 1;
            }
            self.pre.push_back(e);
        }
    }
    fn snapshot(&mut self, at: u64, text: &str) {
        self.tick(at);
        if self.frozen.is_some() {
            return;
        }
        let mut end = text.len().min(SNAP_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end != text.len() {
            self.truncated += 1;
        }
        let sample = (at, text[..end].to_owned());
        if self
            .first_video
            .is_none_or(|t| at.saturating_sub(t) <= INITIAL_US)
            && self.initial_snapshots.len() < 4
        {
            self.initial_snapshots.push(sample.clone());
        }
        if self.trigger.is_some() {
            if self.post_snapshots.len() < 12 {
                self.post_snapshots.push(sample);
            }
        } else {
            if self.snapshots.len() == SNAP_CAP {
                self.snapshots.pop_front();
            }
            self.snapshots.push_back(sample);
        }
    }
    fn finish(&mut self, at: u64) {
        self.tick(at);
        if self.frozen.is_none() {
            self.frozen = Some(at);
            self.reason = if self.trigger.is_some() {
                "CAPTURE INCOMPLETE: early exit"
            } else {
                "NOT REPRODUCED DURING CAPTURE"
            };
        }
    }
    fn save_to(&self, destination: &Path, skipped: u64) -> io::Result<()> {
        // The completed bundle is immutable, including across application restarts.
        // A failed/incomplete write never replaces a successful capture.
        if destination.join("manifest.json").exists() {
            return Ok(());
        }
        let pending = destination.with_extension("partial");
        if let Some(parent) = pending.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::create_dir(&pending)?; // Never overwrite even an earlier partial incident.
        let mut out = BufWriter::new(std::fs::File::create(pending.join("events.csv"))?);
        writeln!(
            out,
            "section,at_us,stage,epoch,ssrc,rtp,seq,media,flags,a,b,c"
        )?;
        for (section, events) in [
            ("initial", self.initial.iter().collect::<Vec<_>>()),
            ("onset", self.pre.iter().collect()),
            ("post", self.post.iter().collect()),
        ] {
            for e in events {
                writeln!(
                    out,
                    "{section},{},{},{},{},{},{},{},{},{},{},{}",
                    e.at,
                    e.stage,
                    e.epoch,
                    e.id.ssrc,
                    e.id.timestamp,
                    e.id.sequence,
                    e.id.media,
                    e.id.flags,
                    e.a,
                    e.b,
                    e.c
                )?;
            }
        }
        out.flush()?;
        out.get_ref().sync_all()?;
        drop(out);
        let mut history = BufWriter::new(std::fs::File::create(pending.join("history.txt"))?);
        for (at, text) in self
            .initial_snapshots
            .iter()
            .chain(self.snapshots.iter())
            .chain(self.post_snapshots.iter())
        {
            writeln!(history, "elapsed_us={at}\n{text}\n")?;
        }
        history.flush()?;
        history.get_ref().sync_all()?;
        drop(history);
        let optional = |x: Option<u64>| x.map_or_else(|| "null".into(), |x| x.to_string());
        let manifest = format!(
            "{{\"schema\":1,\"label\":\"{LABEL}\",\"source\":\"{}\",\"build\":\"{}\",\"outcome\":\"{}\",\"first_video_us\":{},\"trigger_us\":{},\"frozen_us\":{},\"truncated_records\":{},\"lock_skips\":{},\"record_bytes\":{},\"capacity_records\":{},\"actual_packet_arrival_available\":false,\"dequeue_behavior_base\":\"49e6ec12c5b17e95da358f83df34164e0e93a345\"}}\n",
            crate::build_info::REVISION,
            crate::build_info::NUMBER,
            self.reason,
            optional(self.first_video),
            optional(self.trigger),
            optional(self.frozen),
            self.truncated,
            skipped,
            std::mem::size_of::<Event>(),
            CAP * 2 + INITIAL_CAP
        );
        std::fs::write(
            pending.join("README.txt"),
            include_str!("../investigation/DIAGNOSTIC-SCHEMA.txt"),
        )?;
        let mut file = std::fs::File::create(pending.join("manifest.json"))?;
        file.write_all(manifest.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(pending, destination)
    }
}

fn destination() -> PathBuf {
    PathBuf::from(format!(
        "ux0:data/green-vita-540-test/latency-{}",
        crate::build_info::NUMBER
    ))
}

pub(crate) fn begin_stream() {
    display_probe::begin();
    let Ok(mut state) = CAPTURE.lock() else {
        return;
    };
    if state.is_none() {
        if destination().join("manifest.json").exists() {
            STATE.store(7, Ordering::Relaxed);
            return;
        }
        if destination().with_extension("partial").exists() {
            STATE.store(6, Ordering::Relaxed);
            return;
        }
        *state = Some(Capture::new(Instant::now()));
    }
    if let Some(c) = state.as_mut() {
        c.epoch += 1;
        c.clock = TriggerClock::default();
        let at = c.us(Instant::now());
        c.push(Event {
            at,
            stage: "stream_begin",
            epoch: c.epoch,
            id: Identity::default(),
            a: 0,
            b: 0,
            c: 0,
        });
        publish(c);
    }
}
fn publish(c: &Capture) {
    STATE.store(
        if c.reason.starts_with("CAPTURE INCOMPLETE") {
            8
        } else if c.frozen.is_some() {
            if c.trigger.is_some() { 3 } else { 4 }
        } else if c.trigger.is_some() {
            2
        } else {
            1
        },
        Ordering::Relaxed,
    );
}
fn with_capture(f: impl FnOnce(&mut Capture)) {
    if !matches!(STATE.load(Ordering::Relaxed), 1 | 2) {
        return;
    }
    if let Ok(mut state) = CAPTURE.try_lock() {
        if let Some(c) = state.as_mut() {
            f(c);
            publish(c);
        }
    } else {
        SKIPPED.fetch_add(1, Ordering::Relaxed);
    }
}

/// Same monotonic clock for all observations. Times are not wall-clock/NTP dates.
pub(crate) fn packet(
    stage: &'static str,
    id: Identity,
    at: Instant,
    first: Option<Instant>,
    second: Option<Instant>,
    value: u64,
) {
    with_capture(|c| {
        let e = Event {
            at: c.us(at),
            stage,
            epoch: c.epoch,
            id,
            a: first.map_or(UNKNOWN, |t| c.us(t)),
            b: second.map_or(UNKNOWN, |t| c.us(t)),
            c: value,
        };
        c.push(e);
    });
}
pub(crate) fn event(stage: &'static str, timestamp: u32, value: u64) {
    with_capture(|c| {
        c.push(Event {
            at: c.us(Instant::now()),
            stage,
            epoch: c.epoch,
            id: Identity {
                timestamp,
                ..Identity::default()
            },
            a: value,
            b: UNKNOWN,
            c: UNKNOWN,
        })
    });
}
pub(crate) fn track(media: u8, ssrc: u32) {
    with_capture(|c| {
        if media == 1 {
            c.clock = TriggerClock::default();
        }
        c.push(Event {
            at: c.us(Instant::now()),
            stage: "track_open",
            epoch: c.epoch,
            id: Identity {
                ssrc,
                media,
                ..Identity::default()
            },
            a: 0,
            b: 0,
            c: 0,
        });
    });
}
pub(crate) fn snapshot(status: &str) {
    with_capture(|c| c.snapshot(c.us(Instant::now()), status));
}
pub(crate) fn status() -> &'static str {
    display_probe::status()
}
#[allow(dead_code)]
fn latency_status() -> &'static str {
    match STATE.load(Ordering::Relaxed) {
        1 => "Capture: recording (up to 3 minutes)",
        2 => "Capture: delay detected - collecting 10 seconds",
        3 => "Capture complete - exit stream to save",
        4 => "Latency trigger not observed - exit stream to save",
        5 => "Capture saved - ready to copy with VitaShell",
        6 => "Capture SAVE FAILED - check free storage; partial folder retained",
        7 => "Previous capture preserved - copy latency folder with VitaShell",
        8 => "Capture incomplete - exit stream to save available evidence",
        _ => "Diagnostic capture ready",
    }
}
pub(crate) fn save() {
    display_probe::save();
    let capture = {
        let Ok(mut state) = CAPTURE.lock() else {
            return;
        };
        let Some(mut c) = state.take() else {
            return;
        };
        c.finish(c.us(Instant::now()));
        c
    };
    // Worker ownership is ending; no network/decoder work is blocked by disk IO.
    let result = capture.save_to(&destination(), SKIPPED.load(Ordering::Relaxed));
    STATE.store(if result.is_ok() { 5 } else { 6 }, Ordering::Relaxed);
    if let Err(error) = result {
        eprintln!("Diagnostic capture save failed: {error}");
    }
}

#[cfg(test)]
#[path = "diagnostic_tests.rs"]
mod tests;

/// RTP's fixed header is clear under SRTP but is not trusted until RTC authenticates it.
pub(crate) fn udp_identity(bytes: &[u8]) -> Identity {
    if bytes.len() < 12
        || bytes[0] >> 6 != 2
        || (192..=223).contains(&bytes[1])
        || bytes.len() < 12 + usize::from(bytes[0] & 15) * 4
    {
        return Identity::default();
    }
    Identity {
        ssrc: u32::from_be_bytes(bytes[8..12].try_into().expect("checked header")),
        timestamp: u32::from_be_bytes(bytes[4..8].try_into().expect("checked header")),
        sequence: u16::from_be_bytes(bytes[2..4].try_into().expect("checked header")),
        media: 0,
        flags: 1 | ((bytes[1] >> 7) * 2),
    }
}
pub(crate) fn report(ssrc: u32, timestamp: u32, ntp: u64, rate: u32) {
    with_capture(|c| {
        c.push(Event {
            at: c.us(Instant::now()),
            stage: "sender_report",
            epoch: c.epoch,
            id: Identity {
                ssrc,
                timestamp,
                ..Identity::default()
            },
            a: ntp,
            b: u64::from(timestamp),
            c: u64::from(rate),
        })
    });
}
