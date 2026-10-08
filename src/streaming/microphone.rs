//! Session-local microphone state. Mute and final RTP admission share one lock.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const MAX_PENDING_CLIPS: usize = 3;
const MAX_CLIP_AGE: Duration = Duration::from_millis(80);
/// A 20 ms frame whose peak stays under about -54 dBFS counts as quiet.
const QUIET_PEAK: f32 = 0.002;

/// Which Vita audio-in port capture opens. The Vita picks the physical
/// microphone (built-in or headset) itself; the public SceAudioIn API has no
/// device selection. `Raw` opens the unprocessed port instead of the voice
/// chat port, which may or may not be routed differently with a headset.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) enum MicInput {
    #[default]
    Voice,
    Raw,
}
impl MicInput {
    pub(crate) fn from_setting(raw: bool) -> Self {
        if raw { Self::Raw } else { Self::Voice }
    }
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Voice => "voice port",
            Self::Raw => "raw port",
        }
    }
}

#[derive(Default)]
struct State {
    ready: bool,
    negotiated: bool,
    on: bool,
    epoch: u64,
    pending: VecDeque<VoiceClip>,
    peak: f32,
    level_at: Option<Instant>,
    input: MicInput,
    /// What capture actually opened, for the status line and captures.
    opened: Option<String>,
    frames: u64,
    quiet_frames: u64,
    max_peak: f32,
    sent: u64,
    discarded: u64,
    error: Option<String>,
}
#[derive(Clone, Default)]
pub(crate) struct Microphone {
    state: Arc<Mutex<State>>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct CaptureTicket {
    epoch: u64,
}
pub(crate) struct VoiceClip {
    pub ticket: CaptureTicket,
    pub captured_at: Instant,
    pub timestamp: u32,
    pub opus: Vec<u8>,
}

impl Microphone {
    fn lock_state(&self) -> MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                // A sender unwound while holding the admission lock. Never
                // resume capture or reuse an in-flight ticket after that failure.
                let mut state = poisoned.into_inner();
                state.ready = false;
                state.negotiated = false;
                state.on = false;
                state.epoch = state.epoch.wrapping_add(1);
                state.pending.clear();
                state.peak = 0.0;
                state.level_at = None;
                state.error = Some("microphone state interrupted; reconnect to retry".into());
                self.state.clear_poison();
                state
            }
        }
    }

    pub(crate) fn available(&self) -> bool {
        self.lock_state().ready
    }
    pub(crate) fn is_on(&self) -> bool {
        self.lock_state().on
    }
    pub(crate) fn is_active(&self) -> bool {
        let s = self.lock_state();
        s.ready && s.on && s.negotiated
    }
    pub(crate) fn set_ready(&self, ready: bool) {
        let mut s = self.lock_state();
        s.ready = ready;
        s.negotiated = false;
        s.on = false;
        s.epoch = s.epoch.wrapping_add(1);
        if ready {
            s.error = None;
        }
        s.pending.clear();
        s.peak = 0.0;
        s.level_at = None;
    }
    pub(crate) fn fail(&self, error: String) {
        self.set_ready(false);
        self.lock_state().error = Some(error);
    }
    /// An SDP answer opens capture, but never changes the user's mute choice.
    pub(crate) fn set_negotiated(&self, accepted: bool) {
        let mut s = self.lock_state();
        s.negotiated = accepted;
        if !accepted {
            s.epoch = s.epoch.wrapping_add(1);
            s.pending.clear();
            s.peak = 0.0;
            s.level_at = None;
        }
    }
    pub(crate) fn set_on(&self, on: bool) -> bool {
        let mut s = self.lock_state();
        if on && !s.ready {
            return false;
        }
        if s.on != on {
            s.epoch = s.epoch.wrapping_add(1);
            s.pending.clear();
        }
        if on && !s.on {
            s.frames = 0;
            s.quiet_frames = 0;
            s.max_peak = 0.0;
        }
        s.on = on;
        s.peak = 0.0;
        s.level_at = None;
        true
    }
    pub(crate) fn input(&self) -> MicInput {
        self.lock_state().input
    }
    /// Takes effect at once: an open capture ends with its ticket and the
    /// worker reopens on the new port. Mute state is unchanged.
    pub(crate) fn set_input(&self, input: MicInput) {
        let mut s = self.lock_state();
        if s.input == input {
            return;
        }
        s.input = input;
        s.epoch = s.epoch.wrapping_add(1);
        s.pending.clear();
        s.peak = 0.0;
        s.level_at = None;
        s.opened = None;
        s.frames = 0;
        s.quiet_frames = 0;
        s.max_peak = 0.0;
    }
    /// Recorded by the capture worker once its port is open.
    pub(crate) fn set_opened(&self, ticket: CaptureTicket, opened: String) {
        let mut s = self.lock_state();
        if s.epoch == ticket.epoch {
            s.opened = Some(opened);
        }
    }
    pub(crate) fn begin_capture(&self) -> Option<CaptureTicket> {
        let s = self.lock_state();
        (s.ready && s.on && s.negotiated).then_some(CaptureTicket { epoch: s.epoch })
    }
    pub(crate) fn publish(&self, clip: VoiceClip, peak: f32) {
        let mut s = self.lock_state();
        if !s.ready || !s.on || !s.negotiated || s.epoch != clip.ticket.epoch {
            return;
        }
        if s.pending.len() == MAX_PENDING_CLIPS {
            s.pending.pop_front();
            s.discarded += 1;
        }
        s.peak = peak;
        s.level_at = Some(Instant::now());
        s.frames += 1;
        if peak < QUIET_PEAK {
            s.quiet_frames += 1;
        }
        s.max_peak = s.max_peak.max(peak);
        s.pending.push_back(clip);
    }
    /// Called only by the RTC pump. No old clips survive mute/reconnect, and a
    /// stalled sender drops audio rather than accumulating a delayed conversation.
    pub(crate) fn send_pending(&self, mut send: impl FnMut(&VoiceClip) -> bool) {
        let mut s = self.lock_state();
        while let Some(clip) = s.pending.pop_front() {
            if !s.ready
                || !s.on
                || !s.negotiated
                || s.epoch != clip.ticket.epoch
                || clip.captured_at.elapsed() > MAX_CLIP_AGE
            {
                s.discarded += 1;
                continue;
            }
            if send(&clip) {
                s.sent += 1;
            } else {
                s.discarded += 1;
            }
        }
    }
    pub(crate) fn level(&self) -> f32 {
        let s = self.lock_state();
        if s.on
            && s.level_at
                .is_some_and(|t| t.elapsed() < Duration::from_millis(250))
        {
            s.peak
        } else {
            0.0
        }
    }
    pub(crate) fn status(&self) -> String {
        let s = self.lock_state();
        if let Some(error) = &s.error {
            return format!("Mic unavailable: {error}");
        }
        if !s.ready {
            return "Mic: waiting for stream connection".into();
        }
        if !s.on {
            return "Mic off".into();
        }
        if !s.negotiated {
            return "Mic: connecting to Xbox…".into();
        }
        // Local RTP admission is not a remote voice/chat acknowledgement.
        // Input level since the mic was turned on: all-quiet frames with a
        // 0% peak mean the Vita delivered silence from that port.
        format!(
            "Mic on · {} · level max {:.0}% · quiet {}/{} frames · RTP queued {} · dropped {}",
            s.opened.as_deref().unwrap_or(s.input.label()),
            s.max_peak * 100.0,
            s.quiet_frames,
            s.frames,
            s.sent,
            s.discarded
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sender_panic_disables_capture_without_poisoning_ui_or_replaying_voice() {
        let mic = enabled();
        let old = clip(&mic);
        mic.publish(clip(&mic), 1.0);
        mic.publish(clip(&mic), 1.0);
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            mic.send_pending(|_| panic!("injected sender failure"));
        }));
        assert!(failure.is_err());
        assert!(!mic.available());
        assert!(!mic.is_on());
        assert!(!mic.is_active());
        assert_eq!(mic.level(), 0.0);
        assert!(mic.status().contains("state interrupted"));
        assert!(!mic.set_on(true));
        assert!(mic.set_on(false));
        mic.send_pending(|_| panic!("queued voice survived sender failure"));
        mic.set_ready(true);
        mic.set_negotiated(true);
        assert!(mic.begin_capture().is_none());
        mic.set_on(true);
        mic.publish(old, 1.0);
        mic.send_pending(|_| panic!("stale capture ticket survived reinitialization"));
        mic.publish(clip(&mic), 0.5);
        let mut sent = 0;
        mic.send_pending(|_| {
            sent += 1;
            true
        });
        assert_eq!(sent, 1);
    }
    fn clip(mic: &Microphone) -> VoiceClip {
        VoiceClip {
            ticket: mic.begin_capture().unwrap(),
            captured_at: Instant::now(),
            timestamp: 0,
            opus: vec![1],
        }
    }
    fn enabled() -> Microphone {
        let m = Microphone::default();
        m.set_ready(true);
        m.set_negotiated(true);
        m.set_on(true);
        m
    }
    #[test]
    fn capture_waits_for_chat_answer_and_late_answer_does_not_unmute() {
        let mic = Microphone::default();
        mic.set_ready(true);
        mic.set_on(true);
        assert!(mic.begin_capture().is_none());
        assert!(!mic.is_active());
        mic.set_on(false);
        mic.set_negotiated(true);
        assert!(!mic.is_on());
        assert!(mic.begin_capture().is_none());
        mic.set_on(true);
        assert!(mic.begin_capture().is_some());
    }
    #[test]
    fn unavailable_capture_cannot_be_unmuted() {
        let mic = Microphone::default();
        assert!(!mic.set_on(true));
        assert!(mic.begin_capture().is_none());
        mic.set_ready(true);
        assert!(!mic.is_on());
    }
    #[test]
    fn mute_invalidates_inflight_capture_and_queued_audio_even_after_unmute() {
        let mic = enabled();
        let old = clip(&mic);
        mic.publish(clip(&mic), 1.0);
        mic.set_on(false);
        mic.set_on(true);
        mic.publish(old, 1.0);
        mic.send_pending(|_| panic!("old audio leaked"));
        assert_eq!(mic.level(), 0.0);
        mic.publish(clip(&mic), 0.5);
        let mut count = 0;
        mic.send_pending(|_| {
            count += 1;
            true
        });
        assert_eq!(count, 1);
    }
    #[test]
    fn disconnect_and_new_sessions_require_explicit_unmute() {
        let mic = enabled();
        let old = clip(&mic);
        mic.set_ready(false);
        mic.publish(old, 1.0);
        mic.send_pending(|_| panic!("audio after disconnect"));
        mic.set_ready(true);
        assert!(!mic.is_on());
        assert!(!Microphone::default().is_on());
    }
    #[test]
    fn status_reports_input_level_since_the_mic_was_turned_on() {
        let mic = enabled();
        mic.set_opened(mic.begin_capture().unwrap(), "voice port 512@16k".into());
        mic.publish(clip(&mic), 0.0);
        mic.publish(clip(&mic), 0.001);
        mic.publish(clip(&mic), 0.25);
        let status = mic.status();
        assert!(status.contains("voice port 512@16k"), "{status}");
        assert!(status.contains("level max 25%"), "{status}");
        assert!(status.contains("quiet 2/3 frames"), "{status}");
        mic.set_on(false);
        mic.set_on(true);
        assert!(mic.status().contains("quiet 0/0 frames"));
    }
    #[test]
    fn changing_the_input_port_restarts_capture_without_unmuting() {
        let mic = enabled();
        let old = clip(&mic);
        mic.set_opened(old.ticket, "voice port 512@16k".into());
        mic.set_input(MicInput::from_setting(true));
        assert_eq!(mic.input(), MicInput::Raw);
        assert!(mic.is_on());
        // The open voice-port capture loses its ticket, so the worker reopens.
        assert!(mic.begin_capture() != Some(old.ticket));
        mic.set_opened(old.ticket, "stale".into());
        assert!(mic.status().contains("raw port"), "{}", mic.status());
        mic.publish(old, 1.0);
        mic.send_pending(|_| panic!("audio from the old port"));
        mic.set_on(false);
        mic.set_input(MicInput::from_setting(false));
        assert_eq!(mic.input(), MicInput::Voice);
        assert!(!mic.is_on());
    }
    #[test]
    fn congestion_is_bounded_and_expired_audio_never_retries() {
        let mic = enabled();
        for _ in 0..10 {
            mic.publish(clip(&mic), 0.5);
        }
        let mut count = 0;
        mic.send_pending(|_| {
            count += 1;
            false
        });
        assert_eq!(count, 3);
        mic.send_pending(|_| panic!("replayed rejected audio"));
        let mut old = clip(&mic);
        old.captured_at = Instant::now() - Duration::from_secs(1);
        mic.publish(old, 1.0);
        mic.send_pending(|_| panic!("stale audio"));
    }
}
