//! Playback-only gain for the mixed Xbox stream. Never changes microphone gain.
const UNITY: i32 = 100 * 256;
const RAMP_FRAMES: i32 = 480; // 10ms at 48kHz, shared by both stereo channels.

pub(crate) struct AudioGain { current: i32, target: i32, remaining: i32 }
impl Default for AudioGain {
    fn default() -> Self { Self { current: UNITY, target: UNITY, remaining: 0 } }
}
impl AudioGain {
    pub(crate) fn set_percent(&mut self, percent: u8) {
        let target = i32::from(percent.min(100)) * 256;
        if target != self.target { self.target = target; self.remaining = RAMP_FRAMES; }
    }

    pub(crate) fn apply_stereo(&mut self, samples: &mut [i16]) {
        if self.remaining == 0 && self.current == UNITY { return; }
        for frame in samples.chunks_mut(2) {
            if self.remaining > 0 {
                self.current += (self.target - self.current) / self.remaining;
                self.remaining -= 1;
            }
            for sample in frame { *sample = (i32::from(*sample) * self.current / UNITY) as i16; }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gain_is_bounded_and_preserves_stereo_and_full_scale() {
        let mut gain = AudioGain::default();
        let mut pcm = vec![i16::MIN, i16::MAX];
        gain.set_percent(255);
        gain.apply_stereo(&mut pcm);
        assert_eq!(pcm, [i16::MIN, i16::MAX]);
        gain.set_percent(50);
        gain.apply_stereo(&mut vec![0; 960]);
        gain.apply_stereo(&mut pcm);
        assert_eq!(pcm, [-16384, 16383]);
    }
    #[test]
    fn volume_changes_ramp_across_buffers_and_mute_reaches_silence() {
        let mut gain = AudioGain::default();
        gain.set_percent(0);
        let mut first = vec![10000; 480];
        gain.apply_stereo(&mut first);
        assert!(first[0] > 9900 && first[478] >= 4900 && first[478] <= 5100);
        assert!(first.chunks_exact(2).all(|pair| pair[0] == pair[1]));
        gain.set_percent(0); // Applying unchanged settings must not restart the ramp.
        let mut rest = vec![10000; 482];
        gain.apply_stereo(&mut rest);
        assert_eq!(&rest[478..], &[0, 0, 0, 0]);
        gain.set_percent(100);
        gain.apply_stereo(&mut rest);
        gain.apply_stereo(&mut rest);
        assert_eq!(gain.current, UNITY);
    }
}
