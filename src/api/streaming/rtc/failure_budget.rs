//! Bounded retry of pump errors; this does not observe or control media latency.
use std::time::Duration;

#[derive(Default)]
pub(crate) struct FailureBudget(u8);

impl FailureBudget {
    pub(crate) fn recovered(&mut self) -> bool {
        std::mem::replace(&mut self.0, 0) != 0
    }

    pub(crate) fn failed(&mut self) -> Option<Duration> {
        self.0 = self.0.saturating_add(1);
        match self.0 {
            1..=4 => Some(Duration::from_millis(100 << (self.0 - 1))),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistent_failure_exhausts_finite_wait_and_does_not_wrap() {
        let mut budget = FailureBudget::default();
        let mut total = Duration::ZERO;
        let mut retries = 0;
        while let Some(delay) = budget.failed() {
            total += delay;
            retries += 1;
            assert!(retries <= 4);
        }
        assert_eq!(total, Duration::from_millis(1500));
        for _ in 0..1000 {
            assert!(budget.failed().is_none());
        }
    }
    #[test]
    fn success_resets_failure_history() {
        let mut budget = FailureBudget::default();
        assert!(!budget.recovered());
        let first = budget.failed();
        budget.failed();
        assert!(budget.recovered());
        assert_eq!(budget.failed(), first);
    }
}
