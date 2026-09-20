//! One policy for compressed video. Decoded pictures may be replaced freely;
//! compressed reference pictures may not. These are local budgets, not promises
//! about network or Xbox capture latency.
use std::time::{Duration, Instant};

pub(crate) const AU_QUEUE_CAPACITY: usize = 3;
pub(crate) const AU_MAX_AGE: Duration = Duration::from_millis(50);

#[derive(Default)]
pub(crate) struct Recovery {
    waiting: bool,
}

impl Recovery {
    /// Returns true only on the transition: invalidate queued work once.
    pub(crate) fn damage(&mut self) -> bool {
        let changed = !self.waiting;
        self.waiting = true;
        changed
    }

    pub(crate) fn waiting(&self) -> bool { self.waiting }
    pub(crate) fn accepts(&self, idr: bool) -> bool { !self.waiting || idr }

    /// Seeing an IDR is insufficient: it must actually enter the decoder queue.
    pub(crate) fn submitted(&mut self, idr: bool) {
        if idr { self.waiting = false; }
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
