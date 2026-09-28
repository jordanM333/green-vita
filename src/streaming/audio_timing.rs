//! Age follows audio from authenticated RTP reception to playback admission.
//! This is a local residence bound, not capture age or input-to-speaker latency.
use std::time::{Duration, Instant};

pub(crate) const MAX_LOCAL_AUDIO_AGE: Duration = Duration::from_millis(240);

pub(crate) struct TimedAudio<T> {
    pub(crate) data: T,
    pub(crate) received_at: Instant,
    // Independent RTP deadline; None only for sources without media timestamps.
    pub(crate) media_deadline: Option<Instant>,
}

impl<T> TimedAudio<T> {
    pub(crate) fn map<U>(self, data: U) -> TimedAudio<U> {
        TimedAudio {
            data,
            received_at: self.received_at,
            media_deadline: self.media_deadline,
        }
    }

    pub(crate) fn fits_playback(&self, now: Instant, queued: Duration, duration: Duration) -> bool {
        self.media_deadline
            .is_none_or(|deadline| now + queued + duration <= deadline)
            && now
                .saturating_duration_since(self.received_at)
                .saturating_add(queued)
                .saturating_add(duration)
                <= MAX_LOCAL_AUDIO_AGE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presocket_audio_lag_is_not_forgiven_by_decode_or_an_empty_device_queue() {
        let now = Instant::now();
        let old = TimedAudio {
            data: (),
            received_at: now,
            media_deadline: Some(now - Duration::from_millis(1115)),
        };
        assert!(!old.map(vec![0i16; 1920]).fits_playback(
            now,
            Duration::ZERO,
            Duration::from_millis(20)
        ));
        let current = TimedAudio {
            data: (),
            received_at: now,
            media_deadline: Some(now + Duration::from_millis(100)),
        };
        assert!(current.fits_playback(now, Duration::from_millis(80), Duration::from_millis(20)));
        assert!(!current.fits_playback(now, Duration::from_millis(81), Duration::from_millis(20)));
    }
    #[test]
    fn handoffs_and_empty_output_queue_cannot_make_old_audio_fresh() {
        let start = Instant::now();
        let packet = TimedAudio {
            data: vec![1u8],
            received_at: start,
            media_deadline: None,
        };
        let pcm = packet.map(vec![42i16; 1920]);
        assert!(!pcm.fits_playback(
            start + Duration::from_secs(6),
            Duration::ZERO,
            Duration::from_millis(20)
        ));
        assert_eq!(pcm.received_at, start);
    }
    #[test]
    fn admission_includes_audio_already_queued_and_the_entire_new_buffer() {
        let start = Instant::now();
        let pcm = TimedAudio {
            data: (),
            received_at: start,
            media_deadline: None,
        };
        assert!(pcm.fits_playback(
            start + Duration::from_millis(120),
            Duration::from_millis(80),
            Duration::from_millis(40)
        ));
        assert!(!pcm.fits_playback(
            start + Duration::from_millis(121),
            Duration::from_millis(80),
            Duration::from_millis(40)
        ));
    }
}
