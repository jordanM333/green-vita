//! Diagnostic video age. It never initiates a reconnect or a decoder flush.
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(crate) struct VideoTiming {
    pub timestamp: u32,
    pub received_at: Instant,
    pub added_delay_ms: u64,
}

impl VideoTiming {
    pub fn with_presented_frame(self, frame: Option<super::timing::PresentedFrame>, now: Instant) -> Self {
        let Some(frame) = frame else { return self; };
        let age_ms = frame.rendered_at.saturating_duration_since(frame.timing.received_at).as_millis() as u64;
        if now.saturating_duration_since(frame.rendered_at) <= Duration::from_millis(1500)
            && age_ms > self.added_delay_ms
        {
            // Keep the output's own identity/time: repeatedly reading one old
            // rendered frame must not count as continuously observed delay.
            Self { timestamp: frame.timing.rtp_timestamp, received_at: frame.rendered_at, added_delay_ms: age_ms }
        } else { self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slow_decoder_output_is_visible_even_when_rtp_arrival_is_fresh() {
        use super::super::timing::{FrameTiming, PresentedFrame};
        let start = Instant::now();
        let now = start + Duration::from_secs(1);
        let frame = PresentedFrame { timing: FrameTiming { rtp_timestamp: 42, received_at: start,
            submitted_at: start, decoded_at: now, epoch: 0 }, rendered_at: now };
        let arrival = VideoTiming { timestamp: 90_000, received_at: now, added_delay_ms: 5 };
        let measured = arrival.with_presented_frame(Some(frame), now);
        assert_eq!(measured.added_delay_ms, 1000);
        assert_eq!(measured.timestamp, 42);
        assert_eq!(measured.received_at, now);
        let stale = arrival.with_presented_frame(Some(frame), now + Duration::from_secs(2));
        assert_eq!(stale.added_delay_ms, 5);
    }
}
