//! Delay-based receive budget for the negotiated REMB feedback path.
//! Arrival delay is relative to the fastest RTP arrival, not unsynchronized
//! Xbox/Vita wall clocks. This changes sender demand; it never resets playback.
use std::time::{Duration, Instant};

const SAMPLE: Duration = Duration::from_millis(200);
const DECREASE_INTERVAL: Duration = Duration::from_secs(1);
const RECOVERY_INTERVAL: Duration = Duration::from_secs(5);
const SETTLED_RECOVERY_INTERVAL: Duration = Duration::from_secs(10);
const MIN_BPS: u32 = 500_000;

pub(super) struct ReceiveBudget {
    maximum: u32,
    target: u32,
    start: Option<Instant>,
    bytes: u64,
    last_change: Option<Instant>,
    healthy_since: Option<Instant>,
    healthy_floor_ms: u64,
    sample_min_ms: u64,
    delayed_windows: u32,
    pub(super) reductions: u64,
    pub(super) delay_ms: u64,
}

impl ReceiveBudget {
    pub(super) fn new(maximum: u32) -> Self {
        Self { maximum, target: maximum, start: None, bytes: 0,
            last_change: None, healthy_since: None, delayed_windows: 0,
            healthy_floor_ms: 0, sample_min_ms: u64::MAX,
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
        }
        let received_bps = (self.bytes * 8_000 / elapsed.as_millis().max(1) as u64)
            .min(u64::from(u32::MAX)) as u32;
        self.start = Some(now);
        self.bytes = 0;
        self.delay_ms = delay_ms;

        if delay_ms >= 100 {
            self.healthy_since = None;
            self.delayed_windows = self.delayed_windows.saturating_add(1);
            if self.delayed_windows >= 2 && self.last_change.is_none_or(|at|
                now.saturating_duration_since(at) >= DECREASE_INTERVAL)
            {
                // Leave capacity for audio, transport headers and clearing the
                // upstream backlog. Never treat requested bitrate as delivered.
                let next = (self.target * 7 / 10).min((u64::from(received_bps) * 8 / 10) as u32)
                    .max(MIN_BPS.min(self.maximum));
                if next < self.target {
                    self.target = next;
                    self.last_change = Some(now);
                    self.reductions += 1;
                }
            }
        } else {
            self.delayed_windows = 0;
            if delay_ms > 80 {
                self.healthy_since = None;
                return;
            }
            // RX38.9 settled at 59-80ms after a disturbance. Requiring <40ms
            // forever locks quality at the minimum on that now-stable path.
            // Probe slowly only while delay is bounded and not trending up;
            // retain the original baseline and all congestion cut thresholds.
            if self.healthy_since.is_none() || delay_ms > self.healthy_floor_ms + 20 {
                self.healthy_floor_ms = delay_ms;
                self.healthy_since = Some(now);
            }
            self.healthy_floor_ms = self.healthy_floor_ms.min(delay_ms);
            let healthy = *self.healthy_since.get_or_insert(now);
            let interval = if delay_ms <= 40 { RECOVERY_INTERVAL } else { SETTLED_RECOVERY_INTERVAL };
            if now.saturating_duration_since(healthy) >= interval {
                self.target = (self.target + 100_000).min(self.maximum);
                self.last_change = Some(now);
                self.healthy_since = Some(now);
                self.healthy_floor_ms = delay_ms;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_arrival_offset_reproduces_legacy_floor_collapse() {
        // A path offset can remain after a pause without further queue growth.
        // This captures the legacy loop's flaw; TWCC sessions must not run it.
        let start = Instant::now();
        let mut budget = ReceiveBudget::new(2_000_000);
        for tick in 0..=300 {
            budget.receive(25_000, 200, start + Duration::from_millis(tick * 100));
        }
        assert_eq!(budget.target(), MIN_BPS);
        assert!(budget.reductions >= 4);
    }

    #[test]
    fn settled_seventy_ms_path_can_recover_but_large_backlog_cannot() {
        let start = Instant::now();
        let mut budget = ReceiveBudget::new(2_000_000);
        for tick in 0..=150 {
            budget.receive(25_000, 1500, start + Duration::from_millis(tick * 200));
        }
        assert_eq!(budget.target(), MIN_BPS);
        for tick in 151..=201 {
            budget.receive(25_000, 70, start + Duration::from_millis(tick * 200));
        }
        assert_eq!(budget.target(), MIN_BPS + 100_000);
        for tick in 202..=302 {
            budget.receive(25_000, 95, start + Duration::from_millis(tick * 200));
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
    fn backlog_reduces_sender_demand_without_waiting_for_seconds_of_delay() {
        let now = Instant::now();
        let mut budget = ReceiveBudget::new(2_000_000);
        for tick in 0..=10 {
            budget.receive(10_000, if tick < 2 { 0 } else { 150 },
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
        for tick in 31..=60 {
            budget.receive(1_000, 600, now + Duration::from_millis(tick * 100));
        }
        assert_eq!(budget.target(), MIN_BPS);
        for tick in 61..=100 {
            budget.receive(10_000, 5, now + Duration::from_millis(tick * 100));
        }
        assert_eq!(budget.target(), MIN_BPS);
        for tick in 101..=125 {
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
