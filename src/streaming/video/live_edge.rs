//! Relative live-edge contract, independent of the depth of every local queue.
//!
//! This bounds added media lag since the best observed path, NOT absolute capture
//! age. No receiver can promise live video when the sender supplies only old data.
//! Recovery never resets the clock: even an intact IDR can be obsolete.
use std::time::{Duration, Instant};

// Preserve the established 240 ms local budget and allow one such budget for
// ingress variation. The total is a ceiling, never a playout target.
pub(crate) const INGRESS_BUDGET: Duration = super::policy::MAX_LOCAL_VIDEO_AGE;
pub(crate) const MEDIA_BUDGET: Duration = Duration::from_millis(480);
// Two maximum negotiated reorder horizons (60 ms), not one late fragment.
const CONFIRMATION: Duration = Duration::from_millis(120);
// Existing PLI admission cooldown, with bounded exponential retry per incident.
const REQUEST_GAP: Duration = Duration::from_millis(300);
const MAX_REQUESTS: u8 = 3;

/// A modular RTP timeline in monotonic time. Empty probes must not be observed.
#[derive(Clone)]
pub(crate) struct MediaClock {
    rate: u32,
    anchor: Option<(u32, Instant)>,
    last_sequence: Option<u16>,
    last_received: Option<Instant>,
    valid: bool,
    ppm: i64,
    report: Option<(u32, u64, Instant)>,
    rejected_reports: u64,
}

#[cfg(test)]
#[path = "live_edge_tests.rs"]
mod tests;

impl MediaClock {
    pub(crate) fn new(rate: u32) -> Self {
        Self {
            rate,
            anchor: None,
            last_sequence: None,
            last_received: None,
            valid: rate > 0,
            ppm: 0,
            report: None,
            rejected_reports: 0,
        }
    }

    pub(crate) fn valid(&self) -> bool {
        self.valid
    }

    fn duration(&self, ticks: u32) -> Duration {
        // expected() limits the span to 120 seconds; these products fit u64.
        // Avoid a software 128-bit divide per packet on Cortex-A9.
        let nominal_ns = u64::from(ticks) * 1_000_000_000 / u64::from(self.rate);
        Duration::from_nanos(nominal_ns * 1_000_000 / (1_000_000 + self.ppm) as u64)
    }

    pub(crate) fn expected(&self, timestamp: u32) -> Option<Instant> {
        if !self.valid {
            return None;
        }
        let (reference, at) = self.anchor?;
        let ticks = timestamp.wrapping_sub(reference) as i32;
        if ticks.unsigned_abs() > self.rate.saturating_mul(120) {
            return None;
        }
        if ticks >= 0 {
            at.checked_add(self.duration(ticks as u32))
        } else {
            at.checked_sub(self.duration(ticks.unsigned_abs()))
        }
    }

    /// Returns true only for a distinct forward media timestamp.
    pub(crate) fn observe(&mut self, timestamp: u32, sequence: u16, at: Instant) -> bool {
        if !self.valid {
            return false;
        }
        if let Some(previous) = self.last_sequence {
            let forward = sequence.wrapping_sub(previous);
            if forward == 0 || forward >= 1 << 15 {
                return false;
            }
        }
        self.last_sequence = Some(sequence);
        let Some((previous, _)) = self.anchor else {
            self.anchor = Some((timestamp, at));
            self.last_received = Some(at);
            return true;
        };
        let forward = timestamp.wrapping_sub(previous);
        if forward == 0 {
            return false;
        }
        if forward >= 1 << 31 {
            // Small backward timestamps may be reordered media. A large jump on
            // advancing packet sequence is not permission to invent a new edge.
            if previous.wrapping_sub(timestamp) > self.rate * 2 {
                self.valid = false;
            }
            return false;
        }
        let Some(expected) = self.expected(timestamp) else {
            self.valid = false;
            return false;
        };
        // Fast catch-up can deliver many media ticks in a single socket burst.
        // An earlier arrival improves the baseline; it is not a discontinuity.
        // Earlier delivery improves the path baseline; later delivery NEVER
        // moves it forward. Pause, recovery, stale IDRs and shallow queues cannot
        // forgive accumulated delay. RTP elapsed time handles legitimate pauses.
        self.anchor = Some((timestamp, expected.min(at)));
        self.last_received = Some(at);
        true
    }

    pub(crate) fn delay(&self, timestamp: u32, now: Instant) -> Option<Duration> {
        Some(now.saturating_duration_since(self.expected(timestamp)?))
    }

    pub(crate) fn deadline(&self, timestamp: u32) -> Option<Instant> {
        self.expected(timestamp)?.checked_add(MEDIA_BUDGET)
    }

    /// Slow oscillator calibration from independent, SSRC-matched SRs, only
    /// while the media path is healthy. This does not use the Vita wall clock.
    /// A 30-second baseline separates ordinary report jitter from clock rate;
    /// corrections outside +/-1000 ppm remain untrusted, never a clock rebase.
    pub(crate) fn sender_report(&mut self, timestamp: u32, ntp: u64, at: Instant) {
        let Some((old_rtp, old_ntp, old_at)) = self.report else {
            self.report = Some((timestamp, ntp, at));
            return;
        };
        let ntp_delta = ntp.wrapping_sub(old_ntp);
        let rtp_delta = timestamp.wrapping_sub(old_rtp);
        if ntp_delta == 0 || ntp_delta >= 1 << 63 || rtp_delta >= 1 << 31 {
            return;
        }
        let ntp_us = ((u128::from(ntp_delta) * 1_000_000) >> 32) as u64;
        let media_us = u64::from(rtp_delta) * 1_000_000 / u64::from(self.rate);
        // Validate the nominal RTP/NTP mapping independently of delivery jitter.
        if ntp_us >= 1_000_000 && media_us.abs_diff(ntp_us) > ntp_us / 1000 + 1000 {
            // SRs are optional oscillator-calibration observations. Reject an
            // inconsistent pair, not the independently measured RTP timeline.
            // Otherwise one report could permanently blank healthy video (and
            // mute audio) even though both continue advancing normally.
            self.rejected_reports += 1;
            self.report = Some((timestamp, ntp, at));
            return;
        }
        let wall_us = at.saturating_duration_since(old_at).as_micros() as i128;
        if wall_us < 30_000_000 {
            return;
        }
        self.report = Some((timestamp, ntp, at));
        let healthy = self
            .anchor
            .zip(self.last_received)
            .is_some_and(|((ts, _), received)| {
                at.saturating_duration_since(received) < INGRESS_BUDGET
                    && self
                        .delay(ts, received)
                        .is_some_and(|d| d < INGRESS_BUDGET / 2)
            });
        let ppm = (i128::from(media_us) - wall_us) * 1_000_000 / wall_us;
        if healthy && ppm.abs() <= 1000 {
            self.ppm = ppm as i64;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Unmeasured,
    Live,
    AwaitingKeyframe,
    AwaitingPicture,
    ClockUncertain,
}

pub(crate) struct LiveEdge {
    clock: MediaClock,
    state: State,
    pressure: Option<Instant>,
    request_at: Option<Instant>,
    requests: u8,
    fresh_request: bool,
    pub(crate) incidents: u64,
    pub(crate) recovered: u64,
}

impl Default for LiveEdge {
    fn default() -> Self {
        Self {
            clock: MediaClock::new(90_000),
            state: State::Unmeasured,
            pressure: None,
            request_at: None,
            requests: 0,
            fresh_request: false,
            incidents: 0,
            recovered: 0,
        }
    }
}

impl LiveEdge {
    pub(crate) fn state(&self) -> State {
        self.state
    }

    pub(crate) fn added_delay_ms(&self, ts: u32, now: Instant) -> Option<u64> {
        self.clock.delay(ts, now).map(|d| d.as_millis() as u64)
    }

    pub(crate) fn observe(&mut self, ts: u32, seq: u16, received: Instant, now: Instant) -> bool {
        let advanced = self.clock.observe(ts, seq, received);
        self.evaluate(now, advanced)
    }

    /// The last media timestamp also ages while packets stop or repeat. This
    /// is its deadline, not a periodic flush. A duplicate never moves the edge.
    pub(crate) fn poll(&mut self, now: Instant) -> bool {
        self.evaluate(now, false)
    }

    fn evaluate(&mut self, now: Instant, advanced: bool) -> bool {
        // Parameter/probe/startup packets do not prove that playback exists.
        // The first matched presented picture establishes the relative edge
        // from its ORIGINAL dequeue time, never its render/decode completion.
        if self.state == State::Unmeasured {
            return false;
        }
        if !self.clock.valid() {
            let changed = self.state != State::ClockUncertain;
            self.state = State::ClockUncertain;
            return changed;
        }
        let Some((ts, _)) = self.clock.anchor else {
            return false;
        };
        let delay = self.clock.delay(ts, now).unwrap_or(Duration::MAX);
        if matches!(self.state, State::Live | State::AwaitingPicture) {
            if delay <= INGRESS_BUDGET {
                self.pressure = None;
            } else {
                let since = *self.pressure.get_or_insert(now);
                if delay > MEDIA_BUDGET || now.saturating_duration_since(since) >= CONFIRMATION {
                    self.state = State::AwaitingKeyframe;
                    self.pressure = None;
                    self.request_at = None;
                    self.requests = 0;
                    self.fresh_request = false;
                    self.incidents += 1;
                    return true;
                }
            }
        } else if self.state == State::AwaitingKeyframe && advanced && delay <= INGRESS_BUDGET {
            // If the initial requests were buried in obsolete media, request a
            // replacement once on actual return of current media, not forever.
            if !self.fresh_request && self.requests >= MAX_REQUESTS {
                self.fresh_request = true;
                self.request_at = None;
                self.requests = 0;
            }
        }
        false
    }

    pub(crate) fn sender_report(&mut self, ts: u32, ntp: u64, at: Instant) -> bool {
        let rejected = self.clock.rejected_reports;
        self.clock.sender_report(ts, ntp, at);
        self.clock.rejected_reports == rejected
    }

    pub(crate) fn recovering(&self) -> bool {
        matches!(
            self.state,
            State::AwaitingKeyframe | State::AwaitingPicture | State::ClockUncertain
        )
    }

    pub(crate) fn useful(&self, ts: u32, now: Instant) -> bool {
        self.state == State::Unmeasured
            || (self.state != State::ClockUncertain
                && self
                    .clock
                    .deadline(ts)
                    .is_some_and(|deadline| now <= deadline))
    }

    pub(crate) fn ingress_useful(&self, ts: u32, now: Instant) -> bool {
        self.useful(ts, now)
            && (!self.recovering()
                || self
                    .clock
                    .delay(ts, now)
                    .is_some_and(|d| d <= INGRESS_BUDGET))
    }

    pub(crate) fn admit(&self, ts: u32, idr_with_parameters: bool, now: Instant) -> bool {
        self.ingress_useful(ts, now)
            && (self.state != State::AwaitingKeyframe || idr_with_parameters)
    }

    pub(crate) fn submitted(&mut self, ts: u32, idr_with_parameters: bool, now: Instant) {
        if self.state == State::AwaitingKeyframe && self.admit(ts, idr_with_parameters, now) {
            self.state = State::AwaitingPicture;
        }
    }

    pub(crate) fn can_present(&self, ts: u32, now: Instant) -> bool {
        matches!(
            self.state,
            State::Unmeasured | State::Live | State::AwaitingPicture
        ) && self.useful(ts, now)
    }

    pub(crate) fn presented(&mut self, ts: u32, now: Instant) -> bool {
        if self.state == State::AwaitingPicture && self.can_present(ts, now) {
            self.state = State::Live;
            self.pressure = None;
            self.recovered += 1;
            return true;
        }
        false
    }

    pub(crate) fn establish(&mut self, ts: u32, received: Instant, rendered: Instant) -> bool {
        if self.state != State::Unmeasured
            || rendered.saturating_duration_since(received) > super::policy::MAX_LOCAL_VIDEO_AGE
        {
            return false;
        }
        // This happens once per source, not once per IDR or recovery. Delay
        // before the first playable picture remains unknown, not measured live.
        self.clock = MediaClock::new(90_000);
        self.clock.anchor = Some((ts, received));
        self.clock.last_received = Some(received);
        self.state = State::Live;
        true
    }

    pub(crate) fn damage(&mut self) {
        if self.state == State::AwaitingPicture {
            self.state = State::AwaitingKeyframe;
        }
    }

    pub(crate) fn request_due(&mut self, now: Instant) -> bool {
        if self.state != State::AwaitingKeyframe || self.requests >= MAX_REQUESTS {
            return false;
        }
        let gap = REQUEST_GAP * (1u32 << self.requests.saturating_sub(1));
        if self
            .request_at
            .is_some_and(|last| now.saturating_duration_since(last) < gap)
        {
            return false;
        }
        self.request_at = Some(now);
        self.requests += 1;
        true
    }

    pub(crate) fn summary(&self) -> String {
        let timing = self
            .clock
            .anchor
            .zip(self.clock.last_received)
            .map(|((ts, _), received)| {
                format!(
                    " ingress+{}ms newest+{}ms skew:{}ppm SRignored:{}",
                    self.added_delay_ms(ts, received).unwrap_or(u64::MAX),
                    self.added_delay_ms(ts, Instant::now()).unwrap_or(u64::MAX),
                    self.clock.ppm,
                    self.clock.rejected_reports
                )
            })
            .unwrap_or_default();
        format!(
            "Live edge:{:?}{timing} incidents:{} current-picture-recoveries:{} requests:{}/{} (relative, not capture age)",
            self.state, self.incidents, self.recovered, self.requests, MAX_REQUESTS
        )
    }
}
