//! One policy for compressed video. Decoded pictures may be replaced freely;
//! compressed reference pictures may not. These are local budgets, not promises
//! about network or Xbox capture latency.
use std::time::{Duration, Instant};

// Safety limits, not a playout target. Cloud catch-up arrives in groups of eight
// AUs within a few milliseconds. Keep the complete reference chain while the
// decoder catches up; a six-frame limit repeatedly destroyed healthy chains.
// Both limits apply, including to streams made up of very small AUs.
// HA07: 128, up from 32. When the Xbox drains a backlog it delivers up to
// 2.7 s of video per second (HA06-22), while AVCDEC completes about 95 AUs
// per second. The 32-AU queue overflowed at 195 s and forced a keyframe wait.
pub(crate) const AU_QUEUE_CAPACITY: usize = 128;
pub(crate) const AU_QUEUE_BYTES: usize = 4 * 1024 * 1024;
// How long the displayed picture stays live with nothing newer from the
// decoder (a local stall), and the freshness needed to anchor the live-edge
// clock. Since HA07 it no longer expires pictures that are merely behind.
pub(crate) const MAX_LOCAL_VIDEO_AGE: Duration = Duration::from_millis(240);
// HA07: the longest an AU or decoded picture may wait locally while the
// decoder works through a sender drain. HA06-22's drains left pictures
// 240-330 ms old at output. The 240 ms rule expired every one of them, so the
// screen stopped for up to 0.66 s at a time, and expired AUs forced keyframe
// waits. The live edge's presentation deadline still bounds total lateness.
pub(crate) const MAX_LOCAL_CATCH_UP: Duration = super::live_edge::LAG_CEILING;
pub(crate) const AU_PRESSURE_AGE: Duration = Duration::from_millis(50);

// Metadata is NOT firmware readiness. Prefer a queued AU (which can also return
// a picture) over a speculative empty call. Still service output under sustained
// input if unretired submissions accumulate, so no-picture calls cannot create
// the unbounded output debt seen before RX35. Idle input always permits draining.
pub(crate) const OUTPUT_DEBT_WATERMARK: usize = 8;
pub(crate) fn poll_before_input(queued: usize, pending: usize) -> bool {
    pending != 0 && (queued == 0 || pending >= OUTPUT_DEBT_WATERMARK)
}

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub(crate) struct QueueReservation {
    used: Arc<AtomicUsize>,
    bytes: usize,
}
impl QueueReservation {
    pub(crate) fn acquire(used: &Arc<AtomicUsize>, bytes: usize) -> Option<Self> {
        used.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            n.checked_add(bytes)
                .filter(|total| *total <= AU_QUEUE_BYTES)
        })
        .ok()?;
        Some(Self {
            used: used.clone(),
            bytes,
        })
    }
}
impl Drop for QueueReservation {
    fn drop(&mut self) {
        self.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

#[derive(Default)]
pub(crate) struct Recovery {
    waiting: bool,
    started: Option<Instant>,
    longest_wait: Duration,
    completed: u64,
}

impl Recovery {
    /// A new decoder has neither parameter sets nor a reference picture:
    /// submit nothing before a complete IDR. Not a recovery incident.
    pub(crate) fn awaiting_first_idr() -> Self {
        Self {
            waiting: true,
            ..Self::default()
        }
    }

    /// Waiting because damage invalidated decoded work (not the first IDR).
    pub(crate) fn incident(&self) -> bool {
        self.waiting && self.started.is_some()
    }

    /// Returns true only on the transition: invalidate queued work once.
    pub(crate) fn damage(&mut self) -> bool {
        let changed = !self.waiting;
        self.waiting = true;
        if changed {
            self.started = Some(Instant::now());
        }
        changed
    }

    pub(crate) fn waiting(&self) -> bool {
        self.waiting
    }
    pub(crate) fn accepts(&self, idr: bool) -> bool {
        !self.waiting || idr
    }

    pub(crate) fn wait_ms(&self, now: Instant) -> u64 {
        self.started
            .map(|at| now.saturating_duration_since(at).as_millis() as u64)
            .unwrap_or(0)
    }

    pub(crate) fn summary(&self, now: Instant) -> String {
        let current = self.wait_ms(now);
        format!(
            "Recovery wait:{}ms max:{}ms IDRadmitted:{} (not live-edge proof)",
            current,
            current.max(self.longest_wait.as_millis() as u64),
            self.completed
        )
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
    fn a_new_decoder_waits_for_its_first_idr_without_an_incident() {
        let mut state = Recovery::awaiting_first_idr();
        assert!(!state.accepts(false));
        assert!(state.accepts(true));
        assert!(!state.incident());
        // Damage before the first IDR has no decoded work to invalidate.
        assert!(!state.damage());
        assert!(!state.incident());
        state.submitted(true);
        assert!(!state.waiting());
        assert!(state.summary(Instant::now()).contains("IDRadmitted:0"));
        assert!(state.damage());
        assert!(state.incident());
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
