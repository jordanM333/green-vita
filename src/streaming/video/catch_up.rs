//! Ask for a new random-access point without stopping a still-valid stream.
//! This is a recovery request, not proof that the sender cleared its backlog.
use std::time::{Duration, Instant};

const QUEUED_FRAMES: usize = 8;
const PERSISTENCE: Duration = Duration::from_millis(500);
const MAX_SAMPLE_GAP: Duration = Duration::from_millis(250);
const REQUEST_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(crate) struct CatchUp {
    last_sample: Option<(u32, Instant)>,
    pressure_since: Option<Instant>,
    last_request: Option<Instant>,
    requests: u64,
}

impl CatchUp {
    pub(crate) fn requested(&mut self, now: Instant) {
        self.last_request = Some(now);
        self.pressure_since = None;
        self.requests += 1;
    }

    pub(crate) fn requests(&self) -> u64 { self.requests }

    /// Only distinct, advancing frames count. A paused game, repeated status,
    /// brief packet burst, or an existing damage recovery cannot trigger this.
    pub(crate) fn observe(&mut self, timestamp: u32, received_at: Instant,
        _delay_ms: u64, queued: usize, recovering: bool, now: Instant) -> bool
    {
        if recovering || now.saturating_duration_since(received_at) > MAX_SAMPLE_GAP {
            self.pressure_since = None;
            return false;
        }
        if let Some((previous, at)) = self.last_sample {
            let forward = timestamp.wrapping_sub(previous);
            if forward == 0 || forward >= (1 << 31) { return false; }
            if received_at.saturating_duration_since(at) > MAX_SAMPLE_GAP {
                self.pressure_since = None;
            }
        }
        self.last_sample = Some((timestamp, received_at));
        // A keyframe can replace local queued dependencies; it cannot bypass
        // a sender/network queue. Arrival delay by itself caused an IDR every
        // five seconds, adding burst traffic to an already delayed stream.
        if queued < QUEUED_FRAMES {
            self.pressure_since = None;
            return false;
        }
        let since = *self.pressure_since.get_or_insert(received_at);
        if received_at.saturating_duration_since(since) < PERSISTENCE
            || self.last_request.is_some_and(|at|
                now.saturating_duration_since(at) < REQUEST_INTERVAL)
        { return false; }
        self.requested(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upstream_delay_alone_must_not_generate_repeated_keyframe_load() {
        let start = Instant::now();
        let mut state = CatchUp::default();
        let mut requests = Vec::new();
        for tick in 0..=120 {
            let now = start + Duration::from_millis(tick * 100);
            if state.observe((tick * 9000) as u32, now, 1500, 0, false, now) {
                requests.push(tick * 100);
            }
        }
        assert!(requests.is_empty(), "arrival offset with an empty decode queue is not a local catch-up opportunity: {requests:?}");
    }
    #[test]
    fn still_video_and_stale_or_reordered_samples_cannot_trigger_refresh() {
        let start = Instant::now();
        let mut state = CatchUp::default();
        assert!(!state.observe(100, start, 2000, 0, false, start));
        for tick in 1..=100 {
            let now = start + Duration::from_millis(tick * 100);
            assert!(!state.observe(100, start, 2000, 0, false, now));
            assert!(!state.observe(99, now, 2000, 0, false, now));
        }
        let resumed = start + Duration::from_secs(11);
        assert!(!state.observe(200, resumed, 2000, 0, false, resumed));
        assert_eq!(state.requests(), 0);
    }
    #[test]
    fn brief_bursts_and_damage_do_not_count_as_sustained_pressure() {
        let start = Instant::now();
        let mut state = CatchUp::default();
        for tick in 0..=100 {
            let now = start + Duration::from_millis(tick * 100);
            assert!(!state.observe(tick as u32, now, 10,
                if tick % 4 == 0 { 20 } else { 0 }, false, now));
        }
        for tick in 101..=200 {
            let now = start + Duration::from_millis(tick * 100);
            assert!(!state.observe(tick as u32, now, 2000, 32, true, now));
        }
    }
    #[test]
    fn sustained_local_queue_pressure_handles_timestamp_wrap_and_manual_cooldown() {
        let start = Instant::now();
        let mut state = CatchUp::default();
        state.requested(start);
        let mut requested = Vec::new();
        for tick in 0..=60 {
            let now = start + Duration::from_millis(tick * 100);
            if state.observe((u32::MAX - 20).wrapping_add(tick as u32), now,
                5, 16, false, now) { requested.push(tick); }
        }
        assert_eq!(requested, [50]);
    }
}
