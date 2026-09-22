//! Session-local microphone state. Mute and final RTP admission share one lock.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_PENDING_CLIPS: usize = 3;
const MAX_CLIP_AGE: Duration = Duration::from_millis(80);

#[derive(Default)]
struct State {
    ready: bool,
    on: bool,
    epoch: u64,
    pending: VecDeque<VoiceClip>,
    peak: f32,
    level_at: Option<Instant>,
    sent: u64,
    discarded: u64,
    error: Option<String>,
}
#[derive(Clone, Default)]
pub(crate) struct Microphone { state: Arc<Mutex<State>> }
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct CaptureTicket { epoch: u64 }
pub(crate) struct VoiceClip {
    pub ticket: CaptureTicket,
    pub captured_at: Instant,
    pub timestamp: u32,
    pub opus: Vec<u8>,
}

impl Microphone {
    pub(crate) fn available(&self) -> bool { self.state.lock().unwrap().ready }
    pub(crate) fn is_on(&self) -> bool { self.state.lock().unwrap().on }
    pub(crate) fn set_ready(&self, ready: bool) {
        let mut s = self.state.lock().unwrap();
        s.ready = ready; s.on = false; s.epoch = s.epoch.wrapping_add(1);
        if ready { s.error = None; }
        s.pending.clear(); s.peak = 0.0; s.level_at = None;
    }
    pub(crate) fn fail(&self, error: String) {
        self.set_ready(false);
        self.state.lock().unwrap().error = Some(error);
    }
    pub(crate) fn set_on(&self, on: bool) -> bool {
        let mut s = self.state.lock().unwrap();
        if on && !s.ready { return false; }
        if s.on != on { s.epoch = s.epoch.wrapping_add(1); s.pending.clear(); }
        s.on = on; s.peak = 0.0; s.level_at = None; true
    }
    pub(crate) fn begin_capture(&self) -> Option<CaptureTicket> {
        let s = self.state.lock().unwrap();
        (s.ready && s.on).then_some(CaptureTicket { epoch: s.epoch })
    }
    pub(crate) fn publish(&self, clip: VoiceClip, peak: f32) {
        let mut s = self.state.lock().unwrap();
        if !s.ready || !s.on || s.epoch != clip.ticket.epoch { return; }
        if s.pending.len() == MAX_PENDING_CLIPS { s.pending.pop_front(); s.discarded += 1; }
        s.peak = peak; s.level_at = Some(Instant::now()); s.pending.push_back(clip);
    }
    /// Called only by the RTC pump. No old clips survive mute/reconnect, and a
    /// stalled sender drops audio rather than accumulating a delayed conversation.
    pub(crate) fn send_pending(&self, mut send: impl FnMut(&VoiceClip) -> bool) {
        let mut s = self.state.lock().unwrap();
        while let Some(clip) = s.pending.pop_front() {
            if !s.ready || !s.on || s.epoch != clip.ticket.epoch || clip.captured_at.elapsed() > MAX_CLIP_AGE {
                s.discarded += 1; continue;
            }
            if send(&clip) { s.sent += 1; } else { s.discarded += 1; }
        }
    }
    pub(crate) fn level(&self) -> f32 {
        let s = self.state.lock().unwrap();
        if s.on && s.level_at.is_some_and(|t| t.elapsed() < Duration::from_millis(250)) { s.peak } else { 0.0 }
    }
    pub(crate) fn status(&self) -> String {
        let s = self.state.lock().unwrap();
        if let Some(error) = &s.error { return format!("Mic unavailable: {error}"); }
        if !s.ready { return "Mic: waiting for audio uplink".into(); }
        if !s.on { return "Mic off".into(); }
        format!("Mic on · sent {} · dropped {}", s.sent, s.discarded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn clip(mic: &Microphone) -> VoiceClip {
        VoiceClip { ticket: mic.begin_capture().unwrap(), captured_at: Instant::now(), timestamp: 0, opus: vec![1] }
    }
    fn enabled() -> Microphone { let m=Microphone::default(); m.set_ready(true); m.set_on(true); m }
    #[test]
    fn unavailable_capture_cannot_be_unmuted() {
        let mic=Microphone::default(); assert!(!mic.set_on(true)); assert!(mic.begin_capture().is_none());
        mic.set_ready(true); assert!(!mic.is_on());
    }
    #[test]
    fn mute_invalidates_inflight_capture_and_queued_audio_even_after_unmute() {
        let mic=enabled(); let old=clip(&mic); mic.publish(clip(&mic),1.0);
        mic.set_on(false); mic.set_on(true); mic.publish(old,1.0);
        mic.send_pending(|_|panic!("old audio leaked")); assert_eq!(mic.level(),0.0);
        mic.publish(clip(&mic),0.5); let mut count=0;
        mic.send_pending(|_| {count+=1; true}); assert_eq!(count,1);
    }
    #[test]
    fn disconnect_and_new_sessions_require_explicit_unmute() {
        let mic=enabled(); let old=clip(&mic); mic.set_ready(false); mic.publish(old,1.0);
        mic.send_pending(|_|panic!("audio after disconnect")); mic.set_ready(true);
        assert!(!mic.is_on()); assert!(!Microphone::default().is_on());
    }
    #[test]
    fn congestion_is_bounded_and_expired_audio_never_retries() {
        let mic=enabled(); for _ in 0..10 { mic.publish(clip(&mic),0.5); }
        let mut count=0; mic.send_pending(|_| {count+=1; false}); assert_eq!(count,3);
        mic.send_pending(|_|panic!("replayed rejected audio"));
        let mut old=clip(&mic); old.captured_at=Instant::now()-Duration::from_secs(1); mic.publish(old,1.0);
        mic.send_pending(|_|panic!("stale audio"));
    }
}
