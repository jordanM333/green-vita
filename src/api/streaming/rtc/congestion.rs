//! Growth-based REMB fallback (only when TWCC is not active).
//! Absolute arrival offset is not evidence of continuing queue growth. A step
//! after a pause must not be repeatedly spent as new congestion evidence.
use std::time::{Duration, Instant};

const SAMPLE: Duration = Duration::from_millis(200);
const DECREASE_INTERVAL: Duration = Duration::from_secs(1);
const SETTLED_RECOVERY_INTERVAL: Duration = Duration::from_secs(10);
const MIN_BPS: u32 = 500_000;

pub(super) struct ReceiveBudget {
    maximum: u32,
    target: u32,
    start: Option<Instant>,
    bytes: u64,
    last_change: Option<Instant>,
    healthy_since: Option<Instant>,
    sample_min_ms: u64,
    delayed_windows: u32,
    trend: Option<(Instant, u64)>,
    pub(super) reductions: u64,
    pub(super) delay_ms: u64,
}

impl ReceiveBudget {
    pub(super) fn new(maximum: u32) -> Self {
        Self { maximum, target: maximum, start: None, bytes: 0,
            last_change: None, healthy_since: None, delayed_windows: 0,
            trend: None, sample_min_ms: u64::MAX,
            reductions: 0, delay_ms: 0 }
    }

    pub(super) fn target(&self) -> u32 { self.target }

    pub(super) fn receive(&mut self, bytes: usize, delay_ms: u64, now: Instant) {
        self.bytes = self.bytes.saturating_add(bytes as u64);
        self.sample_min_ms = self.sample_min_ms.min(delay_ms);
        let start = *self.start.get_or_insert(now);
        let elapsed = now.saturating_duration_since(start);
        if elapsed < SAMPLE { return; }
        // A single late packet at a window boundary is not persistent backlog.
        let delay_ms = self.sample_min_ms;
        self.sample_min_ms = u64::MAX;
        if elapsed > Duration::from_secs(1) {
            // Time with no video is not evidence of spare bandwidth.
            self.healthy_since = None;
            self.delayed_windows = 0;
            self.trend = None;
        }
        let received_bps = (self.bytes * 8_000 / elapsed.as_millis().max(1) as u64)
            .min(u64::from(u32::MAX)) as u32;
        self.start = Some(now);
        self.bytes = 0;
        self.delay_ms = delay_ms;

        let Some((trend_at, previous_delay)) = self.trend else {
            self.trend = Some((now, delay_ms));
            return;
        };
        if now.saturating_duration_since(trend_at) < DECREASE_INTERVAL { return; }
        self.trend = Some((now, delay_ms));

        // Two successive one-second lower-envelope increases are required.
        // Reuse the existing 20ms jitter tolerance, now against a moving
        // reference rather than the session's fastest-ever arrival.
        if delay_ms > previous_delay.saturating_add(20) {
            self.healthy_since = None;
            self.delayed_windows = self.delayed_windows.saturating_add(1);
            if self.delayed_windows >= 2 && self.last_change.is_none_or(|at|
                now.saturating_duration_since(at) >= DECREASE_INTERVAL)
            {
                // Leave capacity for audio, transport headers and clearing the
                // upstream backlog. Never treat requested bitrate as delivered.
                let next = ((u64::from(self.target) * 7 / 10) as u32).min((u64::from(received_bps) * 8 / 10) as u32)
                    .max(MIN_BPS.min(self.maximum));
                if next < self.target {
                    self.target = next;
                    self.last_change = Some(now);
                    self.reductions += 1;
                }
            }
        } else {
            self.delayed_windows = 0;
            if previous_delay > delay_ms.saturating_add(20) {
                // Queue is draining. Do not immediately undo the reduction.
                self.healthy_since = None;
                return;
            }
            let healthy = *self.healthy_since.get_or_insert(now);
            if now.saturating_duration_since(healthy) >= SETTLED_RECOVERY_INTERVAL {
                self.target = self.target.saturating_add(100_000).min(self.maximum);
                self.last_change = Some(now);
                self.healthy_since = Some(now);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_regression_fixed_offset_does_not_collapse_receive_budget() {
        let start = Instant::now();
        let mut budget = ReceiveBudget::new(2_000_000);
        for tick in 0..=18_000 {
            // Thirty virtual minutes, with an idle-scene/path offset and a
            // small clock skew. Neither is sustained queue growth.
            let delay = if tick < 100 { 5 } else { 200 + tick / 1000 };
            budget.receive(25_000, delay, start + Duration::from_millis(tick * 100));
        }
        assert_eq!(budget.target(), 2_000_000);
        assert_eq!(budget.reductions, 0);
    }

    #[test]
    fn fixed_arrival_offset_does_not_repeat_legacy_floor_collapse() {
        // A path offset can remain after a pause without further queue growth.
        // Applies to fallback too, not just sessions with working TWCC.
        let start = Instant::now();
        let mut budget = ReceiveBudget::new(2_000_000);
        for tick in 0..=300 {
            budget.receive(25_000, 200, start + Duration::from_millis(tick * 100));
        }
        assert_eq!(budget.target(), 2_000_000);
        assert_eq!(budget.reductions, 0);
    }

    #[test]
    fn growing_delay_reduces_but_settled_offset_and_idle_do_not_pin_quality() {
        let start = Instant::now();
        let mut budget = ReceiveBudget::new(2_000_000);
        for tick in 0..=150 {
            budget.receive(25_000, tick * 10, start + Duration::from_millis(tick * 200));
        }
        assert_eq!(budget.target(), MIN_BPS);
        for tick in 151..=211 {
            budget.receive(25_000, 1500, start + Duration::from_millis(tick * 200));
        }
        assert_eq!(budget.target(), MIN_BPS + 100_000);
        budget.receive(25_000, 70, start + Duration::from_secs(80));
        budget.receive(25_000, 70, start + Duration::from_secs(100));
        assert_eq!(budget.target(), MIN_BPS + 100_000, "idle time cannot recover bandwidth");
    }

    #[test]
    fn late_packet_at_every_sample_boundary_does_not_cut_a_clear_path() {
        let start = Instant::now();
        let mut budget = ReceiveBudget::new(2_000_000);
        for tick in 0..=300 {
            budget.receive(12_500, if tick % 2 == 0 { 160 } else { 10 },
                start + Duration::from_millis(tick * 100));
        }
        assert_eq!(budget.target(), 2_000_000);
        assert_eq!(budget.reductions, 0);
    }

    #[test]
    fn sustained_growth_reduces_demand_not_a_single_path_step() {
        let now = Instant::now();
        let mut budget = ReceiveBudget::new(2_000_000);
        for tick in 0..=30 {
            budget.receive(10_000, tick * 4,
                now + Duration::from_millis(tick * 100));
        }
        assert!(budget.target() < 1_000_000);
        assert_eq!(budget.reductions, 1);
    }

    #[test]
    fn brief_jitter_is_ignored_and_recovery_is_deliberate() {
        let now = Instant::now();
        let mut budget = ReceiveBudget::new(2_000_000);
        for tick in 0..=30 {
            budget.receive(25_000, if tick == 2 { 150 } else { 5 },
                now + Duration::from_millis(tick * 100));
        }
        assert_eq!(budget.target(), 2_000_000);
        for tick in 31..=100 {
            budget.receive(1_000, (tick - 30) * 10, now + Duration::from_millis(tick * 100));
        }
        assert_eq!(budget.target(), MIN_BPS);
        for tick in 101..=150 {
            budget.receive(10_000, 5, now + Duration::from_millis(tick * 100));
        }
        assert_eq!(budget.target(), MIN_BPS);
        for tick in 151..=220 {
            budget.receive(10_000, 5, now + Duration::from_millis(tick * 100));
        }
        assert_eq!(budget.target(), MIN_BPS + 100_000);
    }

    #[test]
    fn rate_feedback_clears_a_bandwidth_drop_instead_of_accumulating_delay() {
        // A virtual bottleneck: 2 Mbps fits initially, then capacity drops to
        // 1 Mbps. Feedback reaches the sender after 100 ms. The old fixed cap
        // continues filling the queue; the adaptive budget drains it.
        let start = Instant::now();
        let mut adaptive = ReceiveBudget::new(2_000_000);
        let mut queued_bits = 0f64;
        let mut fixed_bits = 0f64;
        let mut sender_bps = 2_000_000;
        let mut previous_target = sender_bps;
        let mut last_delay = 0;
        for step in 0..600 {
            let capacity: f64 = if step < 20 { 3_000_000.0 } else { 1_000_000.0 };
            queued_bits += f64::from(sender_bps) / 10.0;
            let delivered = queued_bits.min(capacity / 10.0);
            queued_bits -= delivered;
            fixed_bits = (fixed_bits + 200_000.0 - capacity / 10.0).max(0.0);
            last_delay = (queued_bits / capacity * 1_000.0) as u64;
            adaptive.receive((delivered / 8.0) as usize, last_delay,
                start + Duration::from_millis(step * 100));
            sender_bps = previous_target;
            previous_target = adaptive.target();
        }
        assert!(fixed_bits / 1_000_000.0 > 50.0, "fixed budget must reproduce drift");
        assert!(last_delay < 100, "adaptive budget must clear the queue");
        assert!(adaptive.reductions > 0);
    }
}
