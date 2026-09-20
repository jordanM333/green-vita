//! A short sequence-gap grace period, not a playout queue. Ordered packets leave
//! immediately. A later packet must not close the preceding H.264 AU before a
//! reordered tail fragment has had a chance to arrive.
use std::time::{Duration, Instant};

const GAP_GRACE: Duration = Duration::from_millis(6);
const MAX_HELD_PACKETS: usize = 24;

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
}

impl<T> Default for PacketOrder<T> {
    fn default() -> Self {
        Self { next: None, arrival_high: None,
            held: Vec::with_capacity(MAX_HELD_PACKETS), stats: OrderStats::default() }
    }
}

impl<T> PacketOrder<T> {
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
        self.pop(now)
    }

    /// Call on each RTC pump, including pumps without incoming video. The gap
    /// deadline is based on the oldest held packet and is never extended by
    /// newer arrivals. Missing packets are skipped, never fabricated.
    pub(crate) fn pop(&mut self, now: Instant) -> Option<T> {
        let next = self.next?;
        let (index, first) = self.held.iter().enumerate()
            .min_by_key(|(_, p)| p.sequence.wrapping_sub(next))?;
        let missing = first.sequence.wrapping_sub(next);
        if missing != 0 {
            let full = self.held.len() >= MAX_HELD_PACKETS;
            let expired = self.held.iter().any(|p| now.duration_since(p.arrived) >= GAP_GRACE);
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
        format!("Order 6ms fill:{} lost:{} late:{} q:{}/{} cap:{} wait:{}ms",
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
