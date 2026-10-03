//! Relative live-edge measurement and recovery state, independent of the depth
//! of every local queue.
//!
//! This measures added media lag since the best observed path, NOT absolute
//! capture age. HA06: lateness alone no longer stops playback. Video that
//! arrives late (a sender backlog: HA04-20 up to 1.8 s, HA05-21 up to 0.48 s)
//! is decoded and shown, and catches up by itself as the backlog drains. The
//! receiver cannot make queued media current; freezing it only hid video and
//! required keyframes that queue behind the same backlog. Only a broken
//! reference chain (loss, decoder failure) or lateness beyond LAG_CEILING needs
//! a keyframe. Recovery never resets the clock.
use std::time::{Duration, Instant};

// The established healthy-path bound, used to calibrate the clock's rate.
pub(crate) const INGRESS_BUDGET: Duration = super::policy::MAX_LOCAL_VIDEO_AGE;
// Audio's media deadline (MediaClock::deadline). Video uses LAG_CEILING.
pub(crate) const MEDIA_BUDGET: Duration = Duration::from_millis(480);
// Video later than this is not shown and needs a keyframe. It is above the
// most the Xbox has been seen to queue (HA04-20 1.8 s, RX38 2.0 s), so a sender
// backlog alone never reaches it: a safety net for path or clock faults.
pub(crate) const LAG_CEILING: Duration = Duration::from_millis(2_500);
// Existing PLI admission cooldown, shared by every keyframe request path.
pub(crate) const REQUEST_COOLDOWN: Duration = Duration::from_millis(300);
// Recovery requests back off 300 ms -> 600 ms -> 1 s and then repeat at the
// ceiling until decoding resumes. HA03-19 showed a terminal three-request cap
// leaving video black for the rest of a session: the Xbox sends an IDR only on
// request. The ceiling is a local rate limit. No server or protocol limit is
// documented for videoKeyframeRequested or RTCP PLI.
const REQUEST_GAP: Duration = REQUEST_COOLDOWN;
pub(crate) const REQUEST_CEILING: Duration = Duration::from_secs(1);
// No new video for this long while live: the last picture stays held and
// keyframes are requested on the same backoff, without abandoning the
// reference chain (HA05-21: the Xbox sent no video for 3.25 s while a game
// launched, then resumed by itself).
pub(crate) const SILENCE: Duration = REQUEST_CEILING;

/// Wait before the request following `sent` requests in one incident.
fn request_gap(sent: u32) -> Duration {
    REQUEST_GAP
        .saturating_mul(1 << sent.saturating_sub(1).min(8))
        .min(REQUEST_CEILING)
}

/// One keyframe-request decision. Waiting out the backoff is the schedule,
/// not a suppression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Request {
    /// Not recovering; transport/decoder demand decides.
    Idle,
    /// Send now; the next request waits at least this long.
    Send(Duration),
    /// The bounded backoff has not elapsed.
    Wait,
    /// A request is wanted but deliberately not sent.
    Suppressed(Suppression),
}

/// Diagnostic codes recorded as keyframe_request_suppressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Suppression {
    /// The shared cooldown since the previous request (any path).
    Cooldown = 1,
    /// An admitted IDR has REQUEST_CEILING to produce a presented picture.
    AwaitingPicture = 2,
    /// The media clock is untrusted; no keyframe can restore it. Restart.
    ClockUncertain = 3,
    /// The previous keyframe can still be queued behind the sender's backlog:
    /// asked for less than the current video lateness plus the cooldown ago.
    /// HA04-20: keyframes requested into a backlog arrived 0.6-1.7 s later.
    InFlight = 4,
}

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
    request_at: Option<Instant>,
    // Requests in the current incident or silence: the backoff position,
    // never a cap.
    requests: u32,
    // When the recovery IDR entered the decoder (AwaitingPicture only).
    admitted_at: Option<Instant>,
    // First arrival of the most recent media timestamps, for judging a frame
    // by when it started arriving rather than by its own transmission time.
    arrivals: [Option<(u32, Instant)>; 4],
    next_arrival: usize,
    pub(crate) incidents: u64,
    pub(crate) recovered: u64,
    pub(crate) requested: u64,
}

impl Default for LiveEdge {
    fn default() -> Self {
        Self {
            clock: MediaClock::new(90_000),
            state: State::Unmeasured,
            request_at: None,
            requests: 0,
            admitted_at: None,
            arrivals: [None; 4],
            next_arrival: 0,
            incidents: 0,
            recovered: 0,
            requested: 0,
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
        if advanced {
            self.arrivals[self.next_arrival % self.arrivals.len()] = Some((ts, received));
            self.next_arrival = self.next_arrival.wrapping_add(1);
            if self.state == State::Live {
                // Video flows again: a later silence starts a fresh schedule.
                self.request_at = None;
                self.requests = 0;
            }
        }
        self.evaluate(now)
    }

    /// Silence alone is not an incident (see SILENCE). This re-evaluates the
    /// clock and the lag ceiling; it is not a periodic flush.
    pub(crate) fn poll(&mut self, now: Instant) -> bool {
        self.evaluate(now)
    }

    /// Lateness of the newest video when its first packet arrived.
    fn arrival_lateness(&self) -> Option<Duration> {
        let ((ts, _), received) = self.clock.anchor.zip(self.clock.last_received)?;
        self.clock.delay(ts, received)
    }

    /// Lateness of `ts` when its first packet arrived: the queue it waited
    /// behind, not its own transmission. Unknown first arrivals use `now`.
    fn arrival_delay(&self, ts: u32, now: Instant) -> Option<Duration> {
        let first = self
            .arrivals
            .iter()
            .flatten()
            .find(|(timestamp, _)| *timestamp == ts)
            .map_or(now, |(_, at)| (*at).min(now));
        self.clock.delay(ts, first)
    }

    /// How long a keyframe requested now could take to get through the
    /// sender's backlog: the newest video's lateness while video arrives.
    fn in_flight(&self, now: Instant) -> Duration {
        match self.clock.last_received {
            // Whole milliseconds: clock rounding is not a backlog.
            Some(received) if now.saturating_duration_since(received) < SILENCE => {
                self.arrival_lateness().map_or(Duration::ZERO, |late| {
                    Duration::from_millis(late.as_millis() as u64)
                })
            }
            _ => Duration::ZERO,
        }
    }

    fn silent(&self, now: Instant) -> bool {
        self.clock
            .last_received
            .is_some_and(|received| now.saturating_duration_since(received) >= SILENCE)
    }

    fn begin_incident(&mut self) {
        self.state = State::AwaitingKeyframe;
        self.request_at = None;
        self.requests = 0;
        self.admitted_at = None;
        self.incidents += 1;
    }

    fn evaluate(&mut self, _now: Instant) -> bool {
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
        // Late video plays on and catches up as the sender drains; only video
        // beyond the ceiling is abandoned for a keyframe.
        if matches!(self.state, State::Live | State::AwaitingPicture)
            && self
                .arrival_lateness()
                .is_some_and(|late| late > LAG_CEILING)
        {
            self.begin_incident();
            return true;
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
                    .expected(ts)
                    .and_then(|expected| expected.checked_add(LAG_CEILING))
                    .is_some_and(|deadline| now <= deadline))
    }

    /// A frame is judged by when it started arriving, so a keyframe's own
    /// transmission time never counts as lateness.
    pub(crate) fn ingress_useful(&self, ts: u32, now: Instant) -> bool {
        self.state == State::Unmeasured
            || (self.state != State::ClockUncertain
                && self
                    .arrival_delay(ts, now)
                    .is_some_and(|d| d <= LAG_CEILING))
    }

    pub(crate) fn admit(&self, ts: u32, idr_with_parameters: bool, now: Instant) -> bool {
        self.ingress_useful(ts, now)
            && (self.state != State::AwaitingKeyframe || idr_with_parameters)
    }

    pub(crate) fn submitted(&mut self, ts: u32, idr_with_parameters: bool, now: Instant) {
        if self.state == State::AwaitingKeyframe && self.admit(ts, idr_with_parameters, now) {
            self.state = State::AwaitingPicture;
            self.admitted_at = Some(now);
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
            // An admitted IDR produced a current picture: decoding resumed, so
            // the next incident starts its backoff from the shortest gap.
            self.state = State::Live;
            self.request_at = None;
            self.requests = 0;
            self.admitted_at = None;
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
        self.arrivals = [None; 4];
        self.clock.anchor = Some((ts, received));
        self.clock.last_received = Some(received);
        self.state = State::Live;
        true
    }

    /// The reference chain broke (loss, a failed IDR, a decoder deadline).
    pub(crate) fn damage(&mut self) {
        match self.state {
            // An incident: recovery asks on the backoff that never runs out
            // (HA04) and ends with a presented picture.
            State::Live => self.begin_incident(),
            // The admitted IDR failed before a current picture. Keep asking on
            // the incident's existing schedule.
            State::AwaitingPicture => {
                self.state = State::AwaitingKeyframe;
                self.admitted_at = None;
            }
            State::Unmeasured | State::AwaitingKeyframe | State::ClockUncertain => {}
        }
    }

    /// What `request` would do now, without committing a request.
    pub(crate) fn pending_request(&self, now: Instant) -> Request {
        let scheduled = match self.state {
            State::Unmeasured => return Request::Idle,
            State::Live if !self.silent(now) => return Request::Idle,
            State::ClockUncertain => return Request::Suppressed(Suppression::ClockUncertain),
            // Waiting for a keyframe, or live but silent: the bounded backoff.
            State::Live | State::AwaitingKeyframe => true,
            // Give the admitted IDR time to produce a presented picture. If it
            // silently never does, keep asking rather than wait forever.
            State::AwaitingPicture => match self.request_at.max(self.admitted_at) {
                Some(at) if now.saturating_duration_since(at) < REQUEST_CEILING => {
                    return Request::Suppressed(Suppression::AwaitingPicture);
                }
                _ => false,
            },
        };
        if scheduled && let Some(at) = self.request_at {
            let since = now.saturating_duration_since(at);
            if since < request_gap(self.requests) {
                return Request::Wait;
            }
            if since < REQUEST_COOLDOWN + self.in_flight(now) {
                return Request::Suppressed(Suppression::InFlight);
            }
        }
        Request::Send(request_gap(self.requests.saturating_add(1)))
    }

    /// Bounded backoff, never a terminal stop, while video waits for a keyframe.
    pub(crate) fn request(&mut self, now: Instant) -> Request {
        let decision = self.pending_request(now);
        if let Request::Send(_) = decision {
            self.request_at = Some(now);
            self.requests = self.requests.saturating_add(1);
            self.requested = self.requested.saturating_add(1);
        }
        decision
    }

    #[cfg(test)]
    pub(crate) fn request_due(&mut self, now: Instant) -> bool {
        matches!(self.request(now), Request::Send(_))
    }

    /// The single per-pump decision for every keyframe request path. `demand`
    /// is transport/decoder demand, which applies while not recovering;
    /// `last_sent` is the shared cooldown clock (recovery, demand, refresh).
    pub(crate) fn keyframe_request(
        &mut self,
        demand: bool,
        last_sent: Option<Instant>,
        now: Instant,
    ) -> Request {
        let blocked = match last_sent.map(|at| now.saturating_duration_since(at)) {
            Some(since) if since < REQUEST_COOLDOWN => Some(Suppression::Cooldown),
            Some(since) if since < REQUEST_COOLDOWN + self.in_flight(now) => {
                Some(Suppression::InFlight)
            }
            _ => None,
        };
        match (self.pending_request(now), blocked) {
            (Request::Idle, None) if demand => Request::Send(REQUEST_COOLDOWN),
            (Request::Idle, Some(reason)) if demand => Request::Suppressed(reason),
            (Request::Send(_), None) => self.request(now),
            (Request::Send(_), Some(reason)) => Request::Suppressed(reason),
            (decision, _) => decision,
        }
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
            "Live edge:{:?}{timing} incidents:{} current-picture-recoveries:{} requests:{} backoff:{}ms total:{} lag-ceiling:{}ms (relative, not capture age)",
            self.state,
            self.incidents,
            self.recovered,
            self.requests,
            request_gap(self.requests).as_millis(),
            self.requested,
            LAG_CEILING.as_millis()
        )
    }
}
