mod decoder;
mod memory;
pub(crate) mod metrics;
mod worker;

pub const STREAM_WIDTH: u32 = 1280;
pub const STREAM_HEIGHT: u32 = 720;
// Preserve the stock Vita AVC decoder capacity independently of the requested stream size.
pub const HW_DECODER_WIDTH: u32 = 1280;
pub const HW_DECODER_HEIGHT: u32 = 720;
pub const HW_OUTPUT_WIDTH: u32 = 960;
pub const HW_OUTPUT_HEIGHT: u32 = 544;

pub use memory::reserve_decoder_cdram;
pub use metrics::video_performance_summary;
pub use worker::VideoDecodeWorker;
pub(crate) use worker::SubmitResult;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

#[derive(Clone, Copy)]
pub(crate) struct VideoTextureTarget {
    pub(crate) ptr: usize,
    pub(crate) pitch: u32,
    pub(crate) capacity: u32,
}

struct DirectVideoOutputState {
    targets: Option<Vec<VideoTextureTarget>>,
    displayed: Option<usize>,
    pending: Option<(usize, u64, Instant)>,
    next_generation: u64,
}

/// Synchronizes the decoder thread with the SDL/GXM textures owned by the render thread.
/// Pointers are stored as integers so the platform-specific unsafe boundary stays in the code
/// that registers and consumes the textures.
pub(crate) struct DirectVideoOutput {
    state: Mutex<DirectVideoOutputState>,
    pub(crate) decoder_ready: AtomicBool,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl DirectVideoOutput {
    pub(crate) fn new(width: u32, height: u32) -> Self {
        Self {
            state: Mutex::new(DirectVideoOutputState {
                targets: None,
                displayed: None,
                pending: None,
                next_generation: 0,
            }),
            decoder_ready: AtomicBool::new(false),
            width,
            height,
        }
    }

    pub(crate) fn set_targets(&self, targets: Vec<VideoTextureTarget>) {
        if let Ok(mut state) = self.state.lock() {
            state.targets = Some(targets);
            state.displayed = None;
            state.pending = None;
        }
    }

    pub(crate) fn clear_targets(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.targets = None;
            state.displayed = None;
            state.pending = None;
        }
    }

    /// The UI takes the newest completed texture under the same lock used by the decoder.
    /// Passing decoded-frame handles through the RTC and app mailboxes can make them stale
    /// before the UI reads them, causing it to skip a render even when a newer frame is ready.
    pub(crate) fn take_latest_for_display(&self) -> Option<usize> {
        let Ok(mut state) = self.state.lock() else {
            return None;
        };
        let (index, _, decoded_at) = state.pending.take()?;
        state.displayed = Some(index);
        let age_us = decoded_at.elapsed().as_micros() as u64;
        metrics::METRICS.display_age_sum_us.fetch_add(age_us, Ordering::Relaxed);
        metrics::METRICS.display_age_count.fetch_add(1, Ordering::Relaxed);
        metrics::METRICS.display_age_max_us.fetch_max(age_us, Ordering::Relaxed);
        Some(index)
    }

    pub(super) fn lock_decode_target(&self) -> Option<DirectVideoTargetGuard<'_>> {
        let state = self.state.lock().ok()?;
        let targets = state.targets.as_ref()?;
        let pending_index = state.pending.map(|(index, _, _)| index);
        // Decoder reference state must advance even when the UI cannot show every frame.
        // Prefer the spare texture; if the UI is behind, replace the pending frame only.
        // Never decode into the texture currently displayed by the renderer.
        let index = (0..targets.len())
            .find(|index| Some(*index) != state.displayed && Some(*index) != pending_index)
            .or(pending_index.filter(|index| Some(*index) != state.displayed))?;
        let target = *targets.get(index)?;
        Some(DirectVideoTargetGuard {
            state,
            target,
            index,
        })
    }
}

pub(super) struct DirectVideoTargetGuard<'a> {
    state: MutexGuard<'a, DirectVideoOutputState>,
    target: VideoTextureTarget,
    index: usize,
}

impl DirectVideoTargetGuard<'_> {
    pub(super) fn publish(mut self) -> (usize, u64) {
        if self.state.pending.is_some() {
            metrics::METRICS.texture_superseded.fetch_add(1, Ordering::Relaxed);
        }
        self.state.next_generation = self.state.next_generation.wrapping_add(1);
        let generation = self.state.next_generation;
        self.state.pending = Some((self.index, generation, Instant::now()));
        (self.index, generation)
    }
}

pub struct DecodedFrame {
    pub texture_index: usize,
    pub generation: u64,
}

#[derive(Clone, Copy)]
pub struct DecoderConfig {
    pub decode_width: u32,
    pub decode_height: u32,
    pub output_width: u32,
    pub output_height: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_takes_newest_completed_frame_without_waiting_for_rtc_handoff() {
        let output = DirectVideoOutput::new(HW_OUTPUT_WIDTH, HW_OUTPUT_HEIGHT);
        output.set_targets(vec![VideoTextureTarget {
            ptr: 0,
            pitch: 1920,
            capacity: 960 * 544 * 2,
        }; 3]);

        let (first, _) = output.lock_decode_target().unwrap().publish();
        let (second, _) = output.lock_decode_target().unwrap().publish();
        assert_ne!(first, second);
        assert_eq!(output.take_latest_for_display(), Some(second));
        assert_eq!(output.take_latest_for_display(), None);

        let (third, _) = output.lock_decode_target().unwrap().publish();
        assert_ne!(second, third);
        assert_eq!(output.take_latest_for_display(), Some(third));
    }

    #[test]
    fn a_two_texture_fallback_never_decodes_into_the_displayed_texture() {
        let output = DirectVideoOutput::new(HW_OUTPUT_WIDTH, HW_OUTPUT_HEIGHT);
        output.set_targets(vec![VideoTextureTarget {
            ptr: 0,
            pitch: 1920,
            capacity: 960 * 544 * 2,
        }; 2]);

        let (displayed, _) = output.lock_decode_target().unwrap().publish();
        assert_eq!(output.take_latest_for_display(), Some(displayed));
        for _ in 0..4 {
            let (next, _) = output.lock_decode_target().unwrap().publish();
            assert_ne!(next, displayed);
        }
    }
}
