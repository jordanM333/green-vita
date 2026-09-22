//! Match AVCDEC output PTS to submitted AUs; never infer identity from call order.
use std::collections::VecDeque;
use std::time::Instant;

const MAX_TRACKED_PICTURES: usize = 256;
pub(crate) const UNKNOWN_PTS: u64 = u64::MAX;

#[derive(Debug, Clone, Copy)]
pub(crate) struct FrameTiming {
    pub rtp_timestamp: u32,
    pub received_at: Instant,
    pub submitted_at: Instant,
    pub decoded_at: Instant,
    pub epoch: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PresentedFrame {
    pub timing: FrameTiming,
    pub rendered_at: Instant,
}

#[derive(Default)]
pub(crate) struct PictureTracker {
    last_rtp: Option<u32>,
    last_pts: u64,
    pending: VecDeque<(u64, FrameTiming)>,
}

impl PictureTracker {
    /// Unretired metadata is a reason to try output, not a firmware readiness count.
    pub fn has_pending(&self) -> bool { !self.pending.is_empty() }
    pub fn pending_count(&self) -> usize { self.pending.len() }

    pub fn submit(&mut self, rtp: u32, received_at: Instant, submitted_at: Instant, epoch: u64) -> u64 {
        // RTP and Vita AVC timestamps both use 90 kHz. Extend the RTP wrap so
        // pictures on either side retain distinct identities in the decoder.
        self.last_pts = match self.last_rtp {
            Some(last) => self.last_pts + u64::from(rtp.wrapping_sub(last)),
            None => u64::from(rtp),
        };
        self.last_rtp = Some(rtp);
        self.pending.retain(|(pts, _)| *pts != self.last_pts);
        if self.pending.len() == MAX_TRACKED_PICTURES { self.pending.pop_front(); }
        self.pending.push_back((self.last_pts, FrameTiming {
            rtp_timestamp: rtp, received_at, submitted_at, decoded_at: submitted_at, epoch,
        }));
        self.last_pts
    }

    pub fn output(&mut self, pts: u64, decoded_at: Instant) -> Option<FrameTiming> {
        if pts == UNKNOWN_PTS { return None; }
        let index = self.pending.iter().position(|(key, _)| *key == pts)?;
        let (_, mut timing) = self.pending.remove(index)?;
        timing.decoded_at = decoded_at;
        Some(timing)
    }
}

/// A single newest report, never a queue of historical acknowledgements.
#[derive(Default)]
pub(crate) struct PresentationState {
    generation: u64,
    pending: Option<PresentedFrame>,
    pub latest: Option<PresentedFrame>,
}

impl PresentationState {
    pub fn record(&mut self, generation: u64, timing: Option<FrameTiming>, rendered_at: Instant) {
        if generation <= self.generation { return; }
        self.generation = generation;
        self.latest = timing.map(|timing| PresentedFrame { timing, rendered_at });
        self.pending = self.latest;
    }

    pub fn take(&mut self) -> Option<PresentedFrame> { self.pending.take() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn buffered_and_reordered_outputs_keep_their_actual_input_identity() {
        let start = Instant::now();
        let mut tracker = PictureTracker::default();
        let first = tracker.submit(1000, start, start, 1);
        // The first decode call returned no picture. The next may output either AU.
        let second = tracker.submit(2500, start + Duration::from_millis(17), start + Duration::from_millis(18), 1);
        assert_eq!(tracker.output(second, start + Duration::from_millis(20)).unwrap().rtp_timestamp, 2500);
        let delayed = tracker.output(first, start + Duration::from_secs(1)).unwrap();
        assert_eq!(delayed.rtp_timestamp, 1000);
        assert_eq!(delayed.decoded_at.duration_since(delayed.received_at), Duration::from_secs(1));
        assert!(tracker.output(first, start).is_none());
    }

    #[test]
    fn rtp_wrap_and_zero_are_valid_picture_timestamps() {
        let now = Instant::now();
        let mut tracker = PictureTracker::default();
        let before = tracker.submit(u32::MAX - 1499, now, now, 0);
        let after = tracker.submit(0, now, now, 0);
        assert_eq!(after - before, 1500);
        assert_eq!(tracker.output(after, now).unwrap().rtp_timestamp, 0);
        assert_eq!(tracker.output(before, now).unwrap().rtp_timestamp, u32::MAX - 1499);
    }

    #[test]
    fn unknown_unmatched_and_evicted_pts_never_guess_a_picture() {
        let now = Instant::now();
        let mut tracker = PictureTracker::default();
        for rtp in 0..300 { tracker.submit(rtp, now, now, 0); }
        assert_eq!(tracker.pending.len(), MAX_TRACKED_PICTURES);
        assert!(tracker.output(UNKNOWN_PTS, now).is_none());
        assert!(tracker.output(0, now).is_none());
        assert!(tracker.output(9000, now).is_none());
        assert_eq!(tracker.output(299, now).unwrap().rtp_timestamp, 299);
    }

    #[test]
    fn old_epoch_is_preserved_so_recovery_can_reject_delayed_output() {
        let now = Instant::now();
        let mut tracker = PictureTracker::default();
        let old = tracker.submit(100, now, now, 2);
        let new = tracker.submit(200, now, now, 3);
        assert_eq!(tracker.output(old, now).unwrap().epoch, 2);
        assert_eq!(tracker.output(new, now).unwrap().epoch, 3);
    }

    #[test]
    fn only_new_rendered_pictures_generate_feedback_and_pending_reports_coalesce() {
        let now = Instant::now();
        let mut tracker = PictureTracker::default();
        let mut state = PresentationState::default();
        let pts = tracker.submit(10, now, now, 0);
        let frame = tracker.output(pts, now);
        assert!(state.take().is_none()); // Decoding alone is not presentation.
        state.record(1, frame, now);
        assert_eq!(state.take().unwrap().timing.rtp_timestamp, 10);
        state.record(1, frame, now);
        assert!(state.take().is_none()); // Redrawing the same texture is not new video.
        state.record(2, frame, now);
        state.record(3, None, now);
        assert!(state.take().is_none()); // Never reuse identity for an unmatched output.
        assert!(state.latest.is_none());
    }
}
