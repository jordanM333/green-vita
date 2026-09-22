//! Session-local mute gate for the future capture/encoder/RTP sender path.
//! Capability remains unavailable until both capture and negotiated sending work.
//! Tickets prevent audio captured before mute/disconnect from leaking after unmute.
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State { ready: bool, on: bool, epoch: u64 }
#[derive(Clone, Default)]
pub(crate) struct Microphone { state: Arc<Mutex<State>> }
pub(crate) struct CaptureTicket { epoch: u64 }

impl Microphone {
    pub(crate) fn available(&self) -> bool { self.state.lock().unwrap().ready }
    pub(crate) fn is_on(&self) -> bool { self.state.lock().unwrap().on }
    pub(crate) fn set_ready(&self, ready: bool) {
        let mut state=self.state.lock().unwrap();
        // Every transport/capture change invalidates outstanding work and mutes.
        state.ready=ready; state.on=false; state.epoch=state.epoch.wrapping_add(1);
    }
    pub(crate) fn set_on(&self, on: bool) -> bool {
        let mut state=self.state.lock().unwrap();
        if on && !state.ready { return false; }
        if state.on != on { state.epoch=state.epoch.wrapping_add(1); }
        state.on=on; true
    }
    pub(crate) fn begin_capture(&self) -> Option<CaptureTicket> {
        let state=self.state.lock().unwrap();
        (state.ready && state.on).then_some(CaptureTicket{epoch:state.epoch})
    }
    /// The future sender must use this at the final nonblocking RTP admission
    /// point. The lock serializes mute with admission; do not queue raw audio here.
    pub(crate) fn admit(&self, ticket: CaptureTicket, send: impl FnOnce()) -> bool {
        let state=self.state.lock().unwrap();
        if !state.ready || !state.on || state.epoch != ticket.epoch { return false; }
        send(); true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_capture_cannot_be_unmuted() {
        let mic=Microphone::default(); assert!(!mic.set_on(true)); assert!(mic.begin_capture().is_none());
        mic.set_ready(true); assert!(!mic.is_on()); assert!(mic.set_on(true));
        let mut sent=false; assert!(mic.admit(mic.begin_capture().unwrap(),||sent=true)); assert!(sent);
    }
    #[test]
    fn muting_then_unmuting_never_replays_old_audio() {
        let mic=Microphone::default(); mic.set_ready(true); mic.set_on(true);
        let ticket=mic.begin_capture().unwrap(); mic.set_on(false); mic.set_on(true);
        assert!(!mic.admit(ticket,||panic!("stale audio sent")));
    }
    #[test]
    fn disconnect_and_new_sessions_require_explicit_unmute() {
        let mic=Microphone::default(); mic.set_ready(true); mic.set_on(true);
        let ticket=mic.begin_capture().unwrap(); let sender=mic.clone(); mic.set_ready(false);
        assert!(!sender.admit(ticket,||panic!("audio sent after disconnect")));
        mic.set_ready(true); assert!(!mic.is_on()); assert!(!Microphone::default().is_on());
    }
}
