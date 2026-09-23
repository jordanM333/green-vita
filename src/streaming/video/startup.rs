//! A per-session hint, not a decoder reset or an automatic remote-game stop.
use std::time::{Duration, Instant};

pub(crate) const HELP_AFTER: Duration = Duration::from_secs(30);

pub(crate) struct VideoStartup {
    started_at: Instant,
    picture_seen: bool,
}

impl VideoStartup {
    pub(crate) fn new(now: Instant) -> Self { Self { started_at: now, picture_seen: false } }
    pub(crate) fn observe_picture(&mut self, has_picture: bool) { self.picture_seen |= has_picture; }
    pub(crate) fn picture_seen(&self) -> bool { self.picture_seen }
    pub(crate) fn needs_help(&self, now: Instant) -> bool {
        !self.picture_seen && now.saturating_duration_since(self.started_at) >= HELP_AFTER
    }
}

/// Advancing, fresh measurements are required; a stale status snapshot cannot
/// turn one hiccup into a persistent-lag warning. This only offers manual recovery.
#[derive(Default)]
pub(crate) struct VideoLagHelp {
    since: Option<Instant>,
    latest: Option<Instant>,
}
impl VideoLagHelp {
    pub(crate) fn observe(&mut self, measured_at: Instant, delay_ms: u64) {
        if self.latest.is_some_and(|last| measured_at <= last) { return; }
        if self.latest.is_some_and(|last| measured_at.duration_since(last) > Duration::from_millis(1500)) {
            self.since = None;
        }
        self.latest = Some(measured_at);
        if delay_ms >= 500 { self.since.get_or_insert(measured_at); }
        else { self.since = None; }
    }
    pub(crate) fn needs_help(&self, now: Instant) -> bool {
        match (self.since, self.latest) {
            (Some(since), Some(latest)) => latest.duration_since(since) >= Duration::from_secs(3)
                && now.saturating_duration_since(latest) <= Duration::from_millis(1500),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lag_help_requires_sustained_fresh_samples_and_clears_after_recovery() {
        let now = Instant::now();
        let mut lag = VideoLagHelp::default();
        lag.observe(now, 1500);
        lag.observe(now, 1500);
        assert!(!lag.needs_help(now + Duration::from_secs(4)));
        for sec in 1..=3 { lag.observe(now + Duration::from_secs(sec), 1400); }
        assert!(lag.needs_help(now + Duration::from_secs(3)));
        assert!(!lag.needs_help(now + Duration::from_secs(5)));
        lag.observe(now + Duration::from_secs(4), 70);
        assert!(!lag.needs_help(now + Duration::from_secs(4)));
        lag.observe(now + Duration::from_secs(10), 1400);
        assert!(!lag.needs_help(now + Duration::from_secs(10)));
    }
    #[test]
    fn stalled_startup_prompts_once_threshold_passes_and_late_picture_clears_it() {
        let now = Instant::now();
        let mut startup = VideoStartup::new(now);
        assert!(!startup.needs_help(now + HELP_AFTER - Duration::from_millis(1)));
        assert!(startup.needs_help(now + HELP_AFTER));
        startup.observe_picture(true);
        assert!(!startup.needs_help(now + Duration::from_secs(201)));
        startup.observe_picture(false); // Later packet loss is not another startup.
        assert!(!startup.needs_help(now + Duration::from_secs(500)));
        let fresh = VideoStartup::new(now + Duration::from_secs(500));
        assert!(!fresh.needs_help(now + Duration::from_secs(500)));
        assert!(fresh.needs_help(now + Duration::from_secs(530)));
    }
}
