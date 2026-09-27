//! Coalesce clicks only while recovery can still make progress. A missing first
//! picture must not disable Refresh forever. Timeout never destroys a session.
use std::time::{Duration, Instant};

const FIRST_PICTURE_DEADLINE: Duration = Duration::from_secs(15);

#[derive(Default)]
pub(super) struct RefreshAttempt {
    started_at: Option<Instant>,
    awaiting_picture: bool,
}

impl RefreshAttempt {
    pub(super) fn begin(&mut self, now: Instant) -> bool {
        if self.started_at.is_some() {
            return false;
        }
        self.started_at = Some(now);
        self.awaiting_picture = true;
        true
    }
    pub(super) fn awaiting_picture(&self) -> bool {
        self.awaiting_picture
    }
    pub(super) fn finish(&mut self) {
        self.started_at = None;
        self.awaiting_picture = false;
    }
    pub(super) fn failed(&mut self) {
        // Unlock explicit retry, but keep late events from this replacement
        // worker in the recovery path until it actually produces a picture.
        self.started_at = None;
    }
    pub(super) fn timed_out(&mut self, now: Instant) -> bool {
        if self
            .started_at
            .is_some_and(|at| now.saturating_duration_since(at) >= FIRST_PICTURE_DEADLINE)
        {
            self.failed();
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_picture_has_one_timeout_and_allows_an_explicit_retry() {
        let start = Instant::now();
        let mut attempt = RefreshAttempt::default();
        assert!(attempt.begin(start));
        for sec in 0..15 {
            let now = start + Duration::from_secs(sec);
            assert!(!attempt.begin(now));
            assert!(!attempt.timed_out(now));
        }
        assert!(attempt.timed_out(start + FIRST_PICTURE_DEADLINE));
        assert!(attempt.awaiting_picture());
        assert!(!attempt.timed_out(start + FIRST_PICTURE_DEADLINE));
        assert!(attempt.begin(start + FIRST_PICTURE_DEADLINE));
        attempt.finish();
        assert!(!attempt.awaiting_picture());
        assert!(!attempt.timed_out(start + FIRST_PICTURE_DEADLINE * 2));
    }
}
