//! Age follows audio from authenticated RTP reception to playback admission.
//! This is a local residence bound, not capture age or input-to-speaker latency.
use std::time::{Duration, Instant};

pub(crate) const MAX_LOCAL_AUDIO_AGE: Duration = Duration::from_millis(240);

pub(crate) struct TimedAudio<T> {
    pub(crate) data: T,
    pub(crate) received_at: Instant,
}

impl<T> TimedAudio<T> {
    pub(crate) fn map<U>(self, data: U) -> TimedAudio<U> {
        TimedAudio { data, received_at: self.received_at }
    }

    pub(crate) fn fits_playback(&self, now: Instant, queued: Duration, duration: Duration) -> bool {
        now.saturating_duration_since(self.received_at)
            .saturating_add(queued).saturating_add(duration) <= MAX_LOCAL_AUDIO_AGE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handoffs_and_empty_output_queue_cannot_make_old_audio_fresh() {
        let start = Instant::now();
        let packet = TimedAudio { data: vec![1u8], received_at: start };
        let pcm = packet.map(vec![42i16; 1920]);
        assert!(!pcm.fits_playback(start + Duration::from_secs(6), Duration::ZERO,
            Duration::from_millis(20)));
        assert_eq!(pcm.received_at, start);
    }
    #[test]
    fn admission_includes_audio_already_queued_and_the_entire_new_buffer() {
        let start = Instant::now();
        let pcm = TimedAudio { data: (), received_at: start };
        assert!(pcm.fits_playback(start + Duration::from_millis(120),
            Duration::from_millis(80), Duration::from_millis(40)));
        assert!(!pcm.fits_playback(start + Duration::from_millis(121),
            Duration::from_millis(80), Duration::from_millis(40)));
    }
}
