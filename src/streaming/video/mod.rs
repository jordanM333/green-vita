mod buffer_contract;
mod decoder;
mod frame_signal;
pub(crate) mod freshness;
pub(crate) mod live_edge;
mod memory;
pub(crate) mod metrics;
pub(crate) mod policy;
pub(crate) mod startup;
pub(crate) mod timing;
pub(crate) mod trace;
mod worker;

pub const STREAM_WIDTH: u32 = 1280;
pub const STREAM_HEIGHT: u32 = 720;
// Preserve the stock Vita AVC decoder capacity independently of the requested stream size.
pub const HW_DECODER_WIDTH: u32 = 1280;
pub const HW_DECODER_HEIGHT: u32 = 720;
pub const HW_OUTPUT_WIDTH: u32 = 960;
pub const HW_OUTPUT_HEIGHT: u32 = 544;

pub(crate) use memory::CdramBlock;
pub(crate) use memory::release_reserved_decoder_cdram;
pub use memory::reserve_decoder_cdram;
pub use metrics::video_performance_summary;
pub(crate) use worker::SubmitResult;
pub use worker::VideoDecodeWorker;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
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
    // Identity of the displayed picture, revalidated by `video_held`.
    displayed_timing: Option<timing::FrameTiming>,
    pending: Option<(usize, u64, Instant, Option<timing::FrameTiming>)>,
    next_generation: u64,
    decoding: Option<usize>,
    minimum_epoch: u64,
    latest_media: Option<(u64, u32)>,
}

/// Synchronizes CDRAM decoder outputs with the SDL/GXM textures owned by the render thread.
/// Pointers are stored as integers so decoding never retains a temporary SDL texture lock.
pub(crate) struct DirectVideoOutput {
    state: Mutex<DirectVideoOutputState>,
    decode_idle: Condvar,
    pub(crate) presentation: Mutex<timing::PresentationState>,
    pub(crate) live_edge: Mutex<live_edge::LiveEdge>,
    frame_signal: frame_signal::FrameSignal,
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
                displayed_timing: None,
                pending: None,
                next_generation: 0,
                decoding: None,
                minimum_epoch: 0,
                latest_media: None,
            }),
            decode_idle: Condvar::new(),
            presentation: Mutex::new(timing::PresentationState::default()),
            live_edge: Mutex::new(live_edge::LiveEdge::default()),
            frame_signal: frame_signal::FrameSignal::default(),
            decoder_ready: AtomicBool::new(false),
            width,
            height,
        }
    }

    pub(crate) fn set_targets(&self, targets: Vec<VideoTextureTarget>) {
        self.replace_targets(Some(targets));
    }

    pub(crate) fn clear_targets(&self) {
        self.replace_targets(None);
    }

    fn replace_targets(&self, targets: Option<Vec<VideoTextureTarget>>) {
        {
            // Teardown must revoke targets even after a panic. The mutex protects only
            // Rust bookkeeping; the lease's Drop always clears decoding before wakeup.
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            // Close admission before waiting: no new lease can race destruction
            // of the CDRAM owned by StreamingSurface::drop.
            state.targets = None;
            state.pending = None;
            self.frame_signal.set_pending(false);
            while state.decoding.is_some() {
                state = self
                    .decode_idle
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
            state.targets = targets;
            state.displayed = None;
            state.displayed_timing = None;
            state.pending = None;
            self.frame_signal.set_pending(false);
        }
    }

    pub(crate) fn invalidate_before_epoch(&self, epoch: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.minimum_epoch = state.minimum_epoch.max(epoch);
        if state
            .pending
            .is_some_and(|(_, _, _, timing)| timing.is_some_and(|t| t.epoch < state.minimum_epoch))
        {
            state.pending = None;
            self.frame_signal.set_pending(false);
        }
    }

    pub(crate) fn has_pending_frame(&self) -> bool {
        // Input/render scheduling must not wait on the decoder's hardware call.
        self.frame_signal.is_pending()
    }

    pub(crate) fn media_useful(&self, timestamp: u32, now: Instant) -> bool {
        self.live_edge
            .lock()
            .is_ok_and(|edge| edge.useful(timestamp, now))
    }

    pub(crate) fn can_present_media(&self, timestamp: u32, now: Instant) -> bool {
        self.live_edge
            .lock()
            .is_ok_and(|edge| edge.can_present(timestamp, now))
    }

    // Shown on screen until HA09; the host tests assert the state through this.
    #[allow(dead_code)]
    pub(crate) fn live_edge_state(&self) -> live_edge::State {
        self.live_edge
            .lock()
            .map_or(live_edge::State::ClockUncertain, |edge| edge.state())
    }

    // Revalidate an already uploaded picture every draw, including its epoch.
    // A texture is not a continuing authorization to display historical video:
    // only a current picture is drawn as live and reported as presented.
    // HA07: a picture goes stale when the decoder has produced nothing newer
    // for MAX_LOCAL_VIDEO_AGE (a local stall), not because catching up made it
    // older than that since its AU completed.
    pub(crate) fn can_draw(&self, timing: timing::FrameTiming, now: Instant) -> bool {
        self.state.lock().is_ok_and(|state| {
            timing.epoch >= state.minimum_epoch
                && now.saturating_duration_since(timing.decoded_at) <= policy::MAX_LOCAL_VIDEO_AGE
                && now.saturating_duration_since(timing.received_at) <= policy::MAX_LOCAL_CATCH_UP
                && self.can_present_media(timing.rtp_timestamp, now)
        })
    }

    /// The displayed picture is no longer current (stall, recovery or epoch
    /// change). The surface holds it on screen as the last good frame, never
    /// re-presented, until a current picture or detach replaces it. Revoking
    /// targets (session end) clears it. No indicator is drawn since HA09; the
    /// host tests assert the hold through this.
    #[allow(dead_code)]
    pub(crate) fn video_held(&self, now: Instant) -> bool {
        let displayed = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.displayed.and(state.displayed_timing));
        displayed.is_some_and(|timing| !self.can_draw(timing, now))
    }

    pub(crate) fn confirm_presentation(&self, frame: timing::PresentedFrame) {
        // Match the active decode epoch before allowing a completion racing a
        // resync to close the new incident. Feedback may still report the fact
        // that the old frame was drawn; that fact is not a recovery declaration.
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if frame.timing.epoch >= state.minimum_epoch
            && let Ok(mut edge) = self.live_edge.lock()
        {
            if edge.establish(
                frame.timing.rtp_timestamp,
                frame.timing.received_at,
                frame.rendered_at,
            ) {
                trace::record(
                    "live_edge_established",
                    frame.timing.rtp_timestamp,
                    frame.timing.epoch,
                );
            } else if edge.presented(frame.timing.rtp_timestamp, frame.rendered_at) {
                trace::record(
                    "live_edge_recovered",
                    frame.timing.rtp_timestamp,
                    frame.timing.epoch,
                );
            }
        }
    }

    pub(crate) fn has_produced_frame(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.next_generation != 0)
    }

    pub(crate) async fn wait_for_frame(&self) {
        self.frame_signal.wait().await;
    }

    /// The UI takes the newest completed buffer under a short ownership lock.
    /// Hardware calls retain an exclusive surface lease, never this mutex.
    /// Passing decoded-frame handles through the RTC and app mailboxes can make them stale
    /// before the UI reads them, causing it to skip a render even when a newer frame is ready.
    pub(crate) fn take_latest_for_display(
        &self,
    ) -> Option<(
        usize,
        VideoTextureTarget,
        u64,
        Instant,
        Option<timing::FrameTiming>,
    )> {
        self.take_latest_at(Instant::now())
    }

    fn take_latest_at(
        &self,
        now: Instant,
    ) -> Option<(
        usize,
        VideoTextureTarget,
        u64,
        Instant,
        Option<timing::FrameTiming>,
    )> {
        let Ok(mut state) = self.state.lock() else {
            return None;
        };
        let (index, generation, decoded_at, timing) = state.pending.take()?;
        self.frame_signal.set_pending(false);
        // The same local limits as can_draw: never select what cannot be drawn.
        if let Some(timing) = timing
            && (timing.epoch < state.minimum_epoch
                || now.saturating_duration_since(timing.decoded_at) > policy::MAX_LOCAL_VIDEO_AGE
                || now.saturating_duration_since(timing.received_at) > policy::MAX_LOCAL_CATCH_UP
                || !self.can_present_media(timing.rtp_timestamp, now))
        {
            metrics::METRICS
                .stale_picture
                .fetch_add(1, Ordering::Relaxed);
            trace::record(
                "presentation_expired",
                timing.rtp_timestamp,
                now.saturating_duration_since(timing.received_at)
                    .as_micros() as u64,
            );
            return None;
        }
        let target = *state.targets.as_ref()?.get(index)?;
        state.displayed = Some(index);
        state.displayed_timing = timing;
        if let Some(timing) = timing {
            trace::record(
                "presentation_selected",
                timing.rtp_timestamp,
                now.saturating_duration_since(timing.received_at)
                    .as_micros() as u64,
            );
        }
        let age_us = decoded_at.elapsed().as_micros() as u64;
        trace::record("texture_take_generation", 0, generation);
        trace::record("texture_wait_us", 0, age_us);
        metrics::METRICS
            .display_age_sum_us
            .fetch_add(age_us, Ordering::Relaxed);
        metrics::METRICS
            .display_age_count
            .fetch_add(1, Ordering::Relaxed);
        metrics::METRICS
            .display_age_max_us
            .fetch_max(age_us, Ordering::Relaxed);
        Some((index, target, generation, decoded_at, timing))
    }

    pub(super) fn lock_decode_target(&self) -> Option<DirectVideoTargetGuard<'_>> {
        let mut state = self.state.lock().ok()?;
        if state.decoding.is_some() {
            return None;
        }
        let targets = state.targets.as_ref()?;
        let pending_index = state.pending.map(|(index, ..)| index);
        // Decoder reference state must advance even when the UI cannot show every frame.
        // Prefer the spare texture; if the UI is behind, replace the pending frame only.
        // Never decode into the texture currently displayed by the renderer.
        let index = (0..targets.len())
            .find(|index| Some(*index) != state.displayed && Some(*index) != pending_index)
            .or(pending_index.filter(|index| Some(*index) != state.displayed))?;
        let target = *targets.get(index)?;
        if pending_index == Some(index) {
            // With two surfaces, withdraw the pending picture before writing
            // over it, including calls that eventually return no picture.
            state.pending = None;
            self.frame_signal.set_pending(false);
            metrics::METRICS
                .texture_superseded
                .fetch_add(1, Ordering::Relaxed);
        }
        state.decoding = Some(index);
        drop(state);
        Some(DirectVideoTargetGuard {
            output: self,
            target,
            index,
        })
    }
}

pub(super) struct DirectVideoTargetGuard<'a> {
    output: &'a DirectVideoOutput,
    target: VideoTextureTarget,
    index: usize,
}

impl DirectVideoTargetGuard<'_> {
    pub(super) fn publish(self, timing: Option<timing::FrameTiming>) -> Option<(usize, u64)> {
        let mut state = self
            .output
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(timing) = timing {
            let age = timing.received_at.elapsed();
            let regressed = state.latest_media.is_some_and(|(epoch, timestamp)| {
                timing.epoch < epoch
                    || (timing.epoch == epoch
                        && (timing.rtp_timestamp.wrapping_sub(timestamp) as i32) <= 0)
            });
            if timing.epoch < state.minimum_epoch
                || regressed
                || age > policy::MAX_LOCAL_CATCH_UP
                || !self
                    .output
                    .can_present_media(timing.rtp_timestamp, Instant::now())
            {
                metrics::METRICS
                    .stale_picture
                    .fetch_add(1, Ordering::Relaxed);
                trace::record(
                    if regressed {
                        "picture_regressed"
                    } else {
                        "picture_expired"
                    },
                    timing.rtp_timestamp,
                    age.as_micros() as u64,
                );
                return None;
            }
            state.latest_media = Some((timing.epoch, timing.rtp_timestamp));
        }
        if state.pending.is_some() {
            metrics::METRICS
                .texture_superseded
                .fetch_add(1, Ordering::Relaxed);
        }
        state.next_generation = state.next_generation.wrapping_add(1);
        let generation = state.next_generation;
        if state.targets.is_some() {
            state.pending = Some((self.index, generation, Instant::now(), timing));
            self.output.frame_signal.set_pending(true);
        }
        let result = Some((self.index, generation));
        drop(state);
        self.output.frame_signal.wake();
        result
    }
}

impl Drop for DirectVideoTargetGuard<'_> {
    fn drop(&mut self) {
        // Releasing a lease cannot skip wakeup on a poisoned bookkeeping mutex.
        let mut state = self
            .output
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.decoding = None;
        self.output.decode_idle.notify_all();
    }
}

/// Notification only; surface ownership and picture identity live in DirectVideoOutput.
pub struct DecodedFrame;

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
        output.set_targets(vec![
            VideoTextureTarget {
                ptr: 0,
                pitch: 1920,
                capacity: 960 * 544 * 2,
            };
            3
        ]);

        let (first, _) = output.lock_decode_target().unwrap().publish(None).unwrap();
        let (second, _) = output.lock_decode_target().unwrap().publish(None).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            output.take_latest_for_display().map(|(index, ..)| index),
            Some(second)
        );
        assert!(output.take_latest_for_display().is_none());

        let (third, _) = output.lock_decode_target().unwrap().publish(None).unwrap();
        assert_ne!(second, third);
        assert_eq!(
            output.take_latest_for_display().map(|(index, ..)| index),
            Some(third)
        );
    }

    #[test]
    fn a_two_texture_fallback_never_decodes_into_the_displayed_texture() {
        let output = DirectVideoOutput::new(HW_OUTPUT_WIDTH, HW_OUTPUT_HEIGHT);
        output.set_targets(vec![
            VideoTextureTarget {
                ptr: 0,
                pitch: 1920,
                capacity: 960 * 544 * 2,
            };
            2
        ]);

        let (displayed, _) = output.lock_decode_target().unwrap().publish(None).unwrap();
        assert_eq!(
            output.take_latest_for_display().map(|(index, ..)| index),
            Some(displayed)
        );
        for _ in 0..4 {
            let (next, _) = output.lock_decode_target().unwrap().publish(None).unwrap();
            assert_ne!(next, displayed);
        }
    }
}
