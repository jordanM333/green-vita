//! Coalesce clicks only while recovery can still make progress. A missing first
//! picture must not disable Refresh forever. Timeout never destroys a session.
use std::time::{Duration, Instant};

const FIRST_PICTURE_DEADLINE: Duration = Duration::from_secs(15);

#[derive(Default)]
pub(super) struct RefreshAttempt(Option<Instant>);

impl RefreshAttempt {
    pub(super) fn begin(&mut self, now: Instant) -> bool {
        if self.0.is_some() {
            return false;
        }
        self.0 = Some(now);
        true
    }
    pub(super) fn active(&self) -> bool {
        self.0.is_some()
    }
    pub(super) fn finish(&mut self) {
        self.0 = None;
    }
    pub(super) fn timed_out(&mut self, now: Instant) -> bool {
        if self
            .0
            .is_some_and(|at| now.saturating_duration_since(at) >= FIRST_PICTURE_DEADLINE)
        {
            self.finish();
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
        assert!(!attempt.timed_out(start + FIRST_PICTURE_DEADLINE));
        assert!(attempt.begin(start + FIRST_PICTURE_DEADLINE));
        attempt.finish();
        assert!(!attempt.active());
        assert!(!attempt.timed_out(start + FIRST_PICTURE_DEADLINE * 2));
    }
}
