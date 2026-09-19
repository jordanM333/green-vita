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

#[derive(Clone, Copy)]
pub(crate) struct VideoTextureTarget {
    pub(crate) ptr: usize,
    pub(crate) pitch: u32,
    pub(crate) capacity: u32,
}

struct DirectVideoOutputState {
    targets: Option<Vec<VideoTextureTarget>>,
    displayed: Option<usize>,
    pending: Option<(usize, u64)>,
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

    pub(crate) fn mark_displayed(&self, index: usize, generation: u64) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.pending != Some((index, generation)) {
            metrics::METRICS.stale_presentation.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        state.displayed = Some(index);
        state.pending = None;
        true
    }

    pub(super) fn lock_decode_target(&self) -> Option<DirectVideoTargetGuard<'_>> {
        let state = self.state.lock().ok()?;
        let targets = state.targets.as_ref()?;
        let pending_index = state.pending.map(|(index, _)| index);
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
        self.state.pending = Some((self.index, generation));
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
    fn stale_presentation_cannot_select_a_texture_replaced_by_decode() {
        let output = DirectVideoOutput::new(HW_OUTPUT_WIDTH, HW_OUTPUT_HEIGHT);
        output.set_targets(vec![VideoTextureTarget {
            ptr: 0,
            pitch: 1920,
            capacity: 960 * 544 * 2,
        }; 3]);

        let (first, first_generation) = output.lock_decode_target().unwrap().publish();
        let (second, second_generation) = output.lock_decode_target().unwrap().publish();
        assert_ne!(first, second);
        assert!(!output.mark_displayed(first, first_generation));
        assert!(output.mark_displayed(second, second_generation));

        let (third, third_generation) = output.lock_decode_target().unwrap().publish();
        assert_ne!(second, third);
        assert!(output.mark_displayed(third, third_generation));
    }

    #[test]
    fn a_two_texture_fallback_never_decodes_into_the_displayed_texture() {
        let output = DirectVideoOutput::new(HW_OUTPUT_WIDTH, HW_OUTPUT_HEIGHT);
        output.set_targets(vec![VideoTextureTarget {
            ptr: 0,
            pitch: 1920,
            capacity: 960 * 544 * 2,
        }; 2]);

        let (displayed, generation) = output.lock_decode_target().unwrap().publish();
        assert!(output.mark_displayed(displayed, generation));
        for _ in 0..4 {
            let (next, _) = output.lock_decode_target().unwrap().publish();
            assert_ne!(next, displayed);
        }
    }
}
