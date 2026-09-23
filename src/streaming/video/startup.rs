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

#[cfg(test)]
mod tests {
    use super::*;
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
