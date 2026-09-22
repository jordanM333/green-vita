//! One policy for compressed video. Decoded pictures may be replaced freely;
//! compressed reference pictures may not. These are local budgets, not promises
//! about network or Xbox capture latency.
use std::time::{Duration, Instant};

// Cloud can deliver four complete AUs in 241us after an IDR. Admit that
// burst plus two slots of headroom without increasing the residence deadline.
pub(crate) const AU_QUEUE_CAPACITY: usize = 6;
pub(crate) const AU_QUEUE_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const AU_MAX_AGE: Duration = Duration::from_millis(50);

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

pub(crate) fn expired(received: Instant, now: Instant) -> bool {
    now.saturating_duration_since(received) > AU_MAX_AGE
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
    fn decode_queue_deadline_is_not_extended_by_newer_frames() {
        let start = Instant::now();
        assert!(!expired(start, start + AU_MAX_AGE));
        assert!(expired(start, start + AU_MAX_AGE + Duration::from_millis(1)));
    }
}
