//! One policy for compressed video. Decoded pictures may be replaced freely;
//! compressed reference pictures may not. These are local budgets, not promises
//! about network or Xbox capture latency.
use std::time::{Duration, Instant};

// Safety limits, not a playout target. Cloud catch-up arrives in groups of eight
// AUs within a few milliseconds. Keep the complete reference chain while the
// decoder catches up; a six-frame limit repeatedly destroyed healthy chains.
// Both limits apply, including to streams made up of very small AUs.
pub(crate) const AU_QUEUE_CAPACITY: usize = 32;
pub(crate) const AU_QUEUE_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const AU_PRESSURE_AGE: Duration = Duration::from_millis(50);

// Metadata is NOT firmware readiness. Prefer a queued AU (which can also return
// a picture) over a speculative empty call. Still service output under sustained
// input if unretired submissions accumulate, so no-picture calls cannot create
// the unbounded output debt seen before RX35. Idle input always permits draining.
pub(crate) const OUTPUT_DEBT_WATERMARK: usize = 8;
pub(crate) fn poll_before_input(queued: usize, pending: usize) -> bool {
    pending != 0 && (queued == 0 || pending >= OUTPUT_DEBT_WATERMARK)
}

use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

pub(crate) struct QueueReservation { used: Arc<AtomicUsize>, bytes: usize }
impl QueueReservation {
    pub(crate) fn acquire(used: &Arc<AtomicUsize>, bytes: usize) -> Option<Self> {
        used.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            n.checked_add(bytes).filter(|total| *total <= AU_QUEUE_BYTES)
        }).ok()?;
        Some(Self { used: used.clone(), bytes })
    }
}
impl Drop for QueueReservation {
    fn drop(&mut self) { self.used.fetch_sub(self.bytes, Ordering::AcqRel); }
}

#[derive(Default)]
pub(crate) struct Recovery {
    waiting: bool,
    started: Option<Instant>,
    longest_wait: Duration,
    completed: u64,
}

impl Recovery {
    /// Returns true only on the transition: invalidate queued work once.
    pub(crate) fn damage(&mut self) -> bool {
        let changed = !self.waiting;
        self.waiting = true;
        if changed { self.started = Some(Instant::now()); }
        changed
    }

    pub(crate) fn waiting(&self) -> bool { self.waiting }
    pub(crate) fn accepts(&self, idr: bool) -> bool { !self.waiting || idr }

    pub(crate) fn wait_ms(&self, now: Instant) -> u64 {
        self.started.map(|at| now.saturating_duration_since(at).as_millis() as u64).unwrap_or(0)
    }

    pub(crate) fn summary(&self, now: Instant) -> String {
        let current = self.wait_ms(now);
        format!("Recovery wait:{}ms max:{}ms completed:{}", current,
            current.max(self.longest_wait.as_millis() as u64), self.completed)
    }

    /// Seeing an IDR is insufficient: it must actually enter the decoder queue.
    pub(crate) fn submitted(&mut self, idr: bool) {
        if idr {
            self.waiting = false;
            if let Some(started) = self.started.take() {
                self.longest_wait = self.longest_wait.max(started.elapsed());
                self.completed += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_transitions_once_and_requires_a_submitted_idr() {
        let mut state = Recovery::default();
        assert!(state.accepts(false));
        assert!(state.damage());
        assert!(!state.damage());
        assert!(!state.accepts(false));
        assert!(state.accepts(true));
        // A rejected or queue-full IDR must not release subsequent P pictures.
        assert!(state.waiting());
        state.submitted(false);
        assert!(state.waiting());
        state.submitted(true);
        assert!(!state.waiting());
    }
    #[test]
    fn input_priority_does_not_disable_idle_or_output_debt_draining() {
        assert!(!poll_before_input(1, 3));
        assert!(!poll_before_input(8, 3));
        assert!(poll_before_input(1, OUTPUT_DEBT_WATERMARK));
        assert!(poll_before_input(0, 1));
        assert!(!poll_before_input(0, 0));
    }
}
