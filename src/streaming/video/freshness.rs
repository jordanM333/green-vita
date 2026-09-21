//! A conservative reconnect guard, not a capture-to-display latency estimate.
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(crate) struct VideoTiming {
    pub timestamp: u32,
    pub received_at: Instant,
    pub added_delay_ms: u64,
}

impl VideoTiming {
    pub fn with_presented_frame(self, frame: Option<super::timing::PresentedFrame>, now: Instant) -> Self {
        let Some(frame) = frame else { return self; };
        let age_ms = frame.rendered_at.saturating_duration_since(frame.timing.received_at).as_millis() as u64;
        if now.saturating_duration_since(frame.rendered_at) <= Duration::from_millis(1500)
            && age_ms > self.added_delay_ms
        {
            // Keep the output's own identity/time: repeatedly reading one old
            // rendered frame must not count as continuously observed delay.
            Self { timestamp: frame.timing.rtp_timestamp, received_at: frame.rendered_at, added_delay_ms: age_ms }
        } else { self }
    }
}

#[derive(Default)]
pub(crate) struct RefreshGuard {
    first_sample: Option<Instant>,
    last_sample: Option<VideoTiming>,
    high_since: Option<Instant>,
    last_refresh: Option<Instant>,
    pub automatic_count: u8,
}

impl RefreshGuard {
    pub fn fallback(&mut self, now: Instant) -> bool {
        if self.automatic_count >= 2 || self.last_refresh.is_some_and(|at|
            now.saturating_duration_since(at) < Duration::from_secs(30)) {
            return false;
        }
        self.automatic_count += 1;
        self.last_refresh = Some(now);
        self.high_since = None;
        true
    }

    pub fn new_connection(&mut self) {
        self.first_sample = None;
        self.last_sample = None;
        self.high_since = None;
    }

    pub fn observe(&mut self, sample: VideoTiming, now: Instant, enabled: bool) -> bool {
        // No decisions from stale UI events, pauses, or a lack of packets.
        if !enabled || now.saturating_duration_since(sample.received_at) > Duration::from_millis(1500) {
            self.high_since = None;
            return false;
        }
        if let Some(last) = self.last_sample {
            if sample.received_at <= last.received_at || sample.timestamp == last.timestamp {
                return false;
            }
            // Missing samples must not count as continuously observed drift.
            if sample.received_at.duration_since(last.received_at) > Duration::from_millis(1500) {
                self.high_since = None;
            }
        }
        self.last_sample = Some(sample);
        let first = *self.first_sample.get_or_insert(sample.received_at);
        let cooling_down = self.last_refresh.is_some_and(|at|
            now.saturating_duration_since(at) < Duration::from_secs(30));
        if self.automatic_count >= 2 || cooling_down
            || sample.received_at.saturating_duration_since(first) < Duration::from_secs(10)
            || sample.added_delay_ms < 300
        {
            self.high_since = None;
            return false;
        }
        let high_since = *self.high_since.get_or_insert(sample.received_at);
        if sample.received_at.saturating_duration_since(high_since) < Duration::from_secs(2) {
            return false;
        }
        self.fallback(now)
    }
}

/// Repair measured decoder backlog without replacing the Xbox/controller session.
/// Network arrival growth and slow GPU work alone must not flush AVC reference state.
#[derive(Default)]
pub(crate) struct DecoderCatchup {
    last_sample: Option<Instant>,
    high_since: Option<Instant>,
    last_attempt: Option<Instant>,
    pending_since: Option<Instant>,
    pending_epoch: u64,
    pub attempts: u32,
    pub completed: u32,
}

impl DecoderCatchup {
    pub fn pending(&self) -> bool { self.pending_since.is_some() }

    pub fn failed(&self, now: Instant) -> bool {
        self.pending_since.is_some_and(|at|
            now.saturating_duration_since(at) >= Duration::from_secs(3))
    }

    pub fn observe(&mut self, frame: Option<super::timing::PresentedFrame>, now: Instant, enabled: bool) -> bool {
        if !enabled { self.high_since = None; return false; }
        let Some(frame) = frame else { self.high_since = None; return false; };
        if now.saturating_duration_since(frame.rendered_at) > Duration::from_millis(250) {
            self.high_since = None;
            return false;
        }
        let decoded_at = frame.timing.decoded_at;
        if self.last_sample.is_some_and(|last| decoded_at <= last) { return false; }
        if self.last_sample.is_some_and(|last|
            decoded_at.saturating_duration_since(last) > Duration::from_millis(250)) {
            self.high_since = None;
        }
        self.last_sample = Some(decoded_at);
        // Time *inside* the decoder, not the age of an incomplete network AU.
        let delay = decoded_at.saturating_duration_since(frame.timing.submitted_at);
        if delay < Duration::from_millis(100) {
            self.high_since = None;
            if self.pending_since.is_some_and(|at| frame.timing.submitted_at >= at)
                && frame.timing.epoch > self.pending_epoch {
                self.pending_since = None;
                self.completed += 1;
            }
            return false;
        }
        if self.pending() || self.last_attempt.is_some_and(|at|
            now.saturating_duration_since(at) < Duration::from_secs(5)) {
            self.high_since = None;
            return false;
        }
        let since = *self.high_since.get_or_insert(decoded_at);
        if decoded_at.saturating_duration_since(since) < Duration::from_millis(500) {
            return false;
        }
        self.high_since = None;
        self.last_attempt = Some(now);
        self.pending_since = Some(now);
        self.pending_epoch = frame.timing.epoch;
        self.attempts += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::timing::{FrameTiming, PresentedFrame};

    fn picture(start: Instant, ms: u64, decoder_ms: u64, epoch: u64) -> PresentedFrame {
        let decoded_at = start + Duration::from_millis(ms);
        let submitted_at = decoded_at - Duration::from_millis(decoder_ms);
        PresentedFrame { timing: FrameTiming { rtp_timestamp: (ms * 90) as u32,
            received_at: submitted_at, submitted_at, decoded_at, epoch }, rendered_at: decoded_at }
    }

    fn catchup(guard: &mut DecoderCatchup, start: Instant, ms: u64, delay: u64, epoch: u64) -> bool {
        let frame = picture(start, ms, delay, epoch);
        guard.observe(Some(frame), frame.rendered_at, true)
    }

    #[test]
    fn sustained_decoder_backlog_repairs_without_spending_reconnect_budget() {
        let start = Instant::now();
        let mut guard = DecoderCatchup::default();
        for ms in (1000..1500).step_by(100) { assert!(!catchup(&mut guard, start, ms, 150, 2)); }
        assert!(catchup(&mut guard, start, 1500, 150, 2));
        assert_eq!(guard.attempts, 1);
        assert!(guard.pending());
        for ms in (1600..2500).step_by(100) { assert!(!catchup(&mut guard, start, ms, 150, 2)); }
        assert_eq!(guard.attempts, 1);
    }

    #[test]
    fn network_and_gpu_delay_alone_do_not_flush_decoder() {
        let start = Instant::now();
        let mut guard = DecoderCatchup::default();
        for ms in (3000..10000).step_by(100) {
            let mut frame = picture(start, ms, 3, 0);
            frame.timing.received_at = frame.timing.submitted_at - Duration::from_secs(1);
            frame.rendered_at += Duration::from_secs(1);
            assert!(!guard.observe(Some(frame), frame.rendered_at, true));
        }
        assert_eq!(guard.attempts, 0);
    }

    #[test]
    fn bursts_pauses_and_missing_samples_cannot_accumulate_into_a_repair() {
        let start = Instant::now();
        let mut guard = DecoderCatchup::default();
        for ms in (1000..3000).step_by(100) {
            assert!(!catchup(&mut guard, start, ms, if ms % 500 == 0 { 3 } else { 150 }, 0));
        }
        for ms in (3000..5000).step_by(100) {
            let frame = picture(start, ms, 150, 0);
            assert!(!guard.observe(Some(frame), frame.rendered_at, false));
        }
        assert!(!catchup(&mut guard, start, 5100, 150, 0));
        assert!(!catchup(&mut guard, start, 5200, 150, 0));
        assert!(!catchup(&mut guard, start, 5600, 150, 0)); // sample gap resets the run
        assert!(!catchup(&mut guard, start, 5700, 150, 0));
        let duplicate = picture(start, 5700, 150, 0);
        assert!(!guard.observe(Some(duplicate), start + Duration::from_millis(5900), true));
        assert!(!guard.observe(Some(duplicate), start + Duration::from_millis(6100), true)); // stale
        assert!(!catchup(&mut guard, start, 6100, 150, 0));
        assert!(!guard.pending());
    }

    #[test]
    fn recovery_requires_fresh_low_delay_output_from_new_epoch_and_has_cooldown() {
        let start = Instant::now();
        let mut guard = DecoderCatchup::default();
        for ms in (1000..1500).step_by(100) { catchup(&mut guard, start, ms, 150, 4); }
        assert!(catchup(&mut guard, start, 1500, 150, 4));
        assert!(!catchup(&mut guard, start, 1501, 3, 5)); // submitted before request
        assert!(guard.pending());
        assert!(!catchup(&mut guard, start, 1600, 3, 4)); // old epoch
        assert!(guard.pending());
        assert!(!catchup(&mut guard, start, 1700, 3, 5));
        assert!(!guard.pending());
        assert_eq!(guard.completed, 1);
        for ms in (1800..7000).step_by(100) { assert!(!catchup(&mut guard, start, ms, 150, 5)); }
        assert!(catchup(&mut guard, start, 7000, 150, 5));
        assert_eq!(guard.attempts, 2);
    }

    #[test]
    fn repair_timeout_is_bounded_even_without_more_video_and_fallback_is_limited() {
        let start = Instant::now();
        let mut guard = DecoderCatchup::default();
        for ms in (1000..=1500).step_by(100) { catchup(&mut guard, start, ms, 150, 0); }
        assert!(!guard.failed(start + Duration::from_millis(4499)));
        assert!(guard.failed(start + Duration::from_millis(4500)));
        let mut reconnect = RefreshGuard::default();
        assert!(reconnect.fallback(start + Duration::from_secs(5)));
        reconnect.new_connection();
        assert!(!reconnect.fallback(start + Duration::from_secs(34)));
        assert!(reconnect.fallback(start + Duration::from_secs(35)));
        assert!(!reconnect.fallback(start + Duration::from_secs(100)));
    }

    fn sample(start: Instant, second: u64, delay: u64) -> VideoTiming {
        VideoTiming { timestamp: (second * 90_000) as u32,
            received_at: start + Duration::from_secs(second), added_delay_ms: delay }
    }
    fn feed(guard: &mut RefreshGuard, start: Instant, second: u64, delay: u64) -> bool {
        let value = sample(start, second, delay);
        guard.observe(value, value.received_at, true)
    }
    #[test]
    fn healthy_stream_and_idle_picture_do_not_reconnect() {
        let start = Instant::now();
        let mut guard = RefreshGuard::default();
        for second in 0..600 { assert!(!feed(&mut guard, start, second, 12)); }
    }
    #[test]
    fn sustained_drift_triggers_after_grace_and_two_seconds() {
        let start = Instant::now();
        let mut guard = RefreshGuard::default();
        for second in 0..12 { assert!(!feed(&mut guard, start, second, 1000)); }
        assert!(feed(&mut guard, start, 12, 1000));
    }
    #[test]
    fn a_burst_that_catches_up_does_not_reconnect() {
        let start = Instant::now();
        let mut guard = RefreshGuard::default();
        for second in 0..10 { assert!(!feed(&mut guard, start, second, 0)); }
        assert!(!feed(&mut guard, start, 10, 600));
        assert!(!feed(&mut guard, start, 11, 20));
        assert!(!feed(&mut guard, start, 12, 600));
        assert!(!feed(&mut guard, start, 13, 20));
    }
    #[test]
    fn stalled_duplicate_and_stale_samples_cannot_trigger() {
        let start = Instant::now();
        let mut guard = RefreshGuard::default();
        for second in 0..11 { assert!(!feed(&mut guard, start, second, 600)); }
        let last = sample(start, 10, 600);
        for second in 11..20 {
            assert!(!guard.observe(last, start + Duration::from_secs(second), true));
        }
        assert!(!feed(&mut guard, start, 20, 600));
    }
    #[test]
    fn reconnect_budget_and_cooldown_survive_new_connections() {
        let start = Instant::now();
        let mut guard = RefreshGuard::default();
        for second in 0..12 { feed(&mut guard, start, second, 600); }
        assert!(feed(&mut guard, start, 12, 600));
        guard.new_connection();
        for second in 13..44 { assert!(!feed(&mut guard, start, second, 600)); }
        assert!(feed(&mut guard, start, 44, 600));
        guard.new_connection();
        for second in 45..200 { assert!(!feed(&mut guard, start, second, 600)); }
        assert_eq!(guard.automatic_count, 2);
    }
    #[test]
    fn disabled_guard_does_not_interrupt_cloud_or_pause_menu() {
        let start = Instant::now();
        let mut guard = RefreshGuard::default();
        for second in 0..100 {
            let value = sample(start, second, 1000);
            assert!(!guard.observe(value, value.received_at, false));
        }
    }

    #[test]
    fn slow_decoder_output_is_visible_even_when_rtp_arrival_is_fresh() {
        use super::super::timing::{FrameTiming, PresentedFrame};
        let start = Instant::now();
        let now = start + Duration::from_secs(1);
        let frame = PresentedFrame { timing: FrameTiming { rtp_timestamp: 42, received_at: start,
            submitted_at: start, decoded_at: now, epoch: 0 }, rendered_at: now };
        let arrival = sample(start, 1, 5);
        let measured = arrival.with_presented_frame(Some(frame), now);
        assert_eq!(measured.added_delay_ms, 1000);
        assert_eq!(measured.timestamp, 42);
        assert_eq!(measured.received_at, now);
        let stale = arrival.with_presented_frame(Some(frame), now + Duration::from_secs(2));
        assert_eq!(stale.added_delay_ms, 5);
    }
}
