//! A short sequence-gap grace period, not a playout queue. Ordered packets leave
//! immediately. A later packet must not close the preceding H.264 AU before a
//! reordered tail fragment has had a chance to arrive.
use std::time::{Duration, Instant};

const GAP_GRACE: Duration = Duration::from_millis(6);
const MAX_HELD_PACKETS: usize = 24;
const REPAIR_PACKETS: usize = 64;
const REPAIR_GRACE: Duration = Duration::from_millis(60);

struct Held<T> {
    sequence: u16,
    value: T,
    arrived: Instant,
}

#[derive(Default)]
pub(crate) struct OrderStats {
    pub jumps: u64,
    pub provisional_missing: u64,
    pub out_of_order: u64,
    pub duplicates: u64,
    pub filled: u64,
    pub missing: u64,
    pub too_late: u64,
    pub capacity_releases: u64,
    pub max_depth: usize,
    pub max_wait_us: u128,
}

pub(crate) struct PacketOrder<T> {
    next: Option<u16>,
    arrival_high: Option<u16>,
    held: Vec<Held<T>>,
    pub stats: OrderStats,
    repair: bool,
    last_nack: Option<Instant>,
}

impl<T> Default for PacketOrder<T> {
    fn default() -> Self {
        Self { next: None, arrival_high: None,
            held: Vec::with_capacity(MAX_HELD_PACKETS), stats: OrderStats::default(),
            repair: false, last_nack: None }
    }
}

impl<T> PacketOrder<T> {
    pub(crate) fn enable_repair(&mut self, enabled: bool) { self.repair = enabled; }

    pub(crate) fn clear(&mut self) {
        self.next = None;
        self.arrival_high = None;
        self.held.clear();
        self.last_nack = None;
    }

    /// Only request holes for packets still retained by this reorder queue.
    /// Two milliseconds absorbs ordinary reordering; retries are 20ms apart.
    /// The 60ms absolute deadline and 64-packet cap never extend on retries.
    pub(crate) fn missing_for_nack(&mut self, now: Instant) -> Vec<u16> {
        if !self.repair || self.last_nack.is_some_and(|at|
            now.saturating_duration_since(at) < Duration::from_millis(20)) { return Vec::new(); }
        let Some(next) = self.next else { return Vec::new(); };
        let Some(oldest) = self.held.iter().map(|p| p.arrived).min() else { return Vec::new(); };
        let age = now.saturating_duration_since(oldest);
        if age < Duration::from_millis(2) || age >= REPAIR_GRACE { return Vec::new(); }
        let distance = self.held.iter().map(|p| p.sequence.wrapping_sub(next)).max().unwrap_or(0);
        if usize::from(distance) > REPAIR_PACKETS { return Vec::new(); }
        let missing: Vec<_> = (0..distance).map(|offset| next.wrapping_add(offset))
            .filter(|seq| !self.held.iter().any(|p| p.sequence == *seq)).collect();
        if !missing.is_empty() { self.last_nack = Some(now); }
        missing
    }

    /// Consume an authenticated video packet, then drain `pop` before pushing
    /// another. The in-order path does not allocate, copy, or wait.
    pub(crate) fn push(&mut self, sequence: u16, value: T, now: Instant) -> Option<T> {
        if let Some(high) = self.arrival_high {
            let distance = sequence.wrapping_sub(high);
            if distance > 0 && distance < 0x8000 {
                if distance > 1 {
                    self.stats.jumps += 1;
                    self.stats.provisional_missing += u64::from(distance - 1);
                }
                self.arrival_high = Some(sequence);
            } else if distance >= 0x8000 {
                self.stats.out_of_order += 1;
            } else {
                self.stats.duplicates += 1;
                return None;
            }
        } else {
            self.arrival_high = Some(sequence);
        }

        let Some(next) = self.next else {
            self.next = Some(sequence.wrapping_add(1));
            return Some(value);
        };
        let distance = sequence.wrapping_sub(next);
        if distance == 0 {
            if !self.held.is_empty() {
                self.stats.filled += 1;
            }
            self.next = Some(sequence.wrapping_add(1));
            return Some(value);
        }
        if distance >= 0x8000 {
            self.stats.too_late += 1;
            return None;
        }
        if self.held.iter().any(|p| p.sequence == sequence) {
            self.stats.duplicates += 1;
            return None;
        }
        self.held.push(Held { sequence, value, arrived: now });
        self.stats.max_depth = self.stats.max_depth.max(self.held.len());
        // Even a stalled consumer cannot grow the queue beyond its fixed bound.
        self.pop_ready(now)
    }

    /// Drain a receive batch without expiring holes ahead of packets that may
    /// already be in that batch. Capacity pressure still releases immediately.
    pub(crate) fn pop_ready(&mut self, now: Instant) -> Option<T> {
        self.take(now, false)
    }

    /// Call on each RTC pump, including pumps without incoming video. The gap
    /// deadline is based on the oldest held packet and is never extended by
    /// newer arrivals. Missing packets are skipped, never fabricated.
    pub(crate) fn pop(&mut self, now: Instant) -> Option<T> {
        self.take(now, true)
    }

    fn take(&mut self, now: Instant, expire_gaps: bool) -> Option<T> {
        let next = self.next?;
        let (index, first) = self.held.iter().enumerate()
            .min_by_key(|(_, p)| p.sequence.wrapping_sub(next))?;
        let missing = first.sequence.wrapping_sub(next);
        if missing != 0 {
            let full = self.held.len() >= if self.repair { REPAIR_PACKETS } else { MAX_HELD_PACKETS };
            let expired = expire_gaps && self.held.iter().any(|p| now.duration_since(p.arrived) >= if self.repair { REPAIR_GRACE } else { GAP_GRACE });
            if !full && !expired {
                return None;
            }
            self.stats.missing += u64::from(missing);
            self.stats.capacity_releases += u64::from(full);
        }
        let packet = self.held.remove(index);
        self.stats.max_wait_us = self.stats.max_wait_us.max(now.duration_since(packet.arrived).as_micros());
        self.next = Some(packet.sequence.wrapping_add(1));
        Some(packet.value)
    }

    pub(crate) fn summary(&self) -> String {
        format!("Order {}ms fill:{} lost:{} late:{} q:{}/{} cap:{} wait:{}ms",
            if self.repair { REPAIR_GRACE.as_millis() } else { GAP_GRACE.as_millis() },
            self.stats.filled, self.stats.missing, self.stats.too_late,
            self.held.len(), self.stats.max_depth, self.stats.capacity_releases,
            self.stats.max_wait_us / 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_packets_are_immediate_and_wrap_correctly() {
        let mut order = PacketOrder::default();
        let now = Instant::now();
        for sequence in [65534, 65535, 0, 1] {
            assert_eq!(order.push(sequence, sequence, now), Some(sequence));
            assert!(order.pop(now).is_none());
        }
        assert_eq!(order.stats.max_depth, 0);
    }

    #[test]
    fn gap_closes_before_deadline_without_losing_or_duplicating_packets() {
        let mut order = PacketOrder::default();
        let now = Instant::now();
        assert_eq!(order.push(65534, 65534, now), Some(65534));
        assert_eq!(order.push(0, 0, now), None);
        assert_eq!(order.push(0, 0, now), None);
        assert_eq!(order.push(65535, 65535, now + Duration::from_millis(1)), Some(65535));
        assert_eq!(order.pop(now + Duration::from_millis(1)), Some(0));
        assert_eq!(order.pop(now + Duration::from_millis(1)), None);
        assert_eq!(order.stats.filled, 1);
        assert_eq!(order.stats.missing, 0);
        assert_eq!(order.stats.duplicates, 1);
    }

    #[test]
    fn new_arrivals_do_not_extend_the_gap_deadline() {
        let mut order = PacketOrder::default();
        let now = Instant::now();
        assert_eq!(order.push(10, 10, now), Some(10));
        assert_eq!(order.push(12, 12, now), None);
        assert_eq!(order.push(13, 13, now + Duration::from_millis(5)), None);
        assert_eq!(order.pop(now + GAP_GRACE), Some(12));
        assert_eq!(order.pop(now + GAP_GRACE), Some(13));
        assert_eq!(order.push(11, 11, now + GAP_GRACE), None);
        assert_eq!(order.stats.missing, 1);
        assert_eq!(order.stats.too_late, 1);
    }

    #[test]
    fn burst_pressure_releases_early_instead_of_growing_the_queue() {
        let mut order = PacketOrder::default();
        let now = Instant::now();
        assert_eq!(order.push(0, 0, now), Some(0));
        for sequence in 2..25 {
            assert_eq!(order.push(sequence, sequence, now), None);
        }
        assert_eq!(order.push(25, 25, now), Some(2));
        for sequence in 3..=25 {
            assert_eq!(order.pop(now), Some(sequence));
        }
        assert_eq!(order.stats.max_depth, MAX_HELD_PACKETS);
        assert_eq!(order.stats.capacity_releases, 1);
        assert_eq!(order.stats.missing, 1);
    }
}

#[cfg(test)]
mod repair_tests {
    use super::*;
    #[test]
    fn nack_repairs_a_wrapped_hole_at_44ms_without_delaying_ordered_packets() {
        let mut q = PacketOrder::default(); q.enable_repair(true);
        let start = Instant::now();
        assert_eq!(q.push(65534, 65534, start), Some(65534));
        assert_eq!(q.push(0, 0, start), None);
        assert!(q.missing_for_nack(start + Duration::from_millis(1)).is_empty());
        assert_eq!(q.missing_for_nack(start + Duration::from_millis(2)), vec![65535]);
        assert!(q.missing_for_nack(start + Duration::from_millis(21)).is_empty());
        assert!(q.pop(start + Duration::from_millis(30)).is_none());
        let repaired = start + Duration::from_millis(46);
        assert_eq!(q.push(65535, 65535, repaired), Some(65535));
        assert_eq!(q.pop_ready(repaired), Some(0));
        assert!(q.missing_for_nack(repaired).is_empty());
        assert_eq!(q.push(1, 1, repaired), Some(1));
        assert_eq!(q.stats.missing, 0);
    }
    #[test]
    fn nack_retry_never_extends_deadline_or_requests_unretained_packets() {
        let mut q = PacketOrder::default(); q.enable_repair(true);
        let t = Instant::now(); q.push(10, 10, t); q.push(13, 13, t);
        for ms in [2, 22, 42] {
            assert_eq!(q.missing_for_nack(t + Duration::from_millis(ms)), vec![11, 12]);
        }
        assert!(q.missing_for_nack(t + REPAIR_GRACE).is_empty());
        assert_eq!(q.pop(t + REPAIR_GRACE), Some(13));
        assert_eq!(q.stats.missing, 2);
        assert!(q.missing_for_nack(t + Duration::from_millis(100)).is_empty());
        q.push(15, 15, t + Duration::from_millis(100)); q.clear();
        assert!(q.missing_for_nack(t + Duration::from_millis(150)).is_empty());
        assert_eq!(q.push(200, 200, t + Duration::from_millis(150)), Some(200));
    }
    #[test]
    fn repair_queue_stays_bounded_on_a_large_burst() {
        let mut q = PacketOrder::default(); q.enable_repair(true);
        let t = Instant::now(); q.push(0, 0, t);
        for seq in 2..=65 { q.push(seq, seq, t); }
        assert_eq!(q.stats.max_depth, 64);
        assert_eq!(q.stats.capacity_releases, 1);
        assert!(q.missing_for_nack(t + Duration::from_millis(2)).is_empty());
    }
}
