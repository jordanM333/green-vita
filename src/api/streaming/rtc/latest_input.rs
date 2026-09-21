//! One unsent controller state, replaceable by a newer sample or pulse release.
#[derive(Default)]
pub(crate) struct LatestInput<T>(Option<T>);

impl<T> LatestInput<T> {
    pub(crate) fn new() -> Self { Self(None) }
    pub(crate) fn replace(&mut self, state: T) { self.0 = Some(state); }
    pub(crate) fn try_send(&mut self, send: impl FnOnce(&T) -> bool) {
        if self.0.as_ref().is_some_and(send) { self.0 = None; }
    }
}

#[cfg(test)]
mod tests {
    use super::LatestInput;
    #[test]
    fn backpressure_retains_release_and_newer_input_replaces_unsent_press() {
        let mut pending = LatestInput::new();
        pending.replace("press");
        pending.try_send(|_| false);
        pending.replace("release");
        for _ in 0..100 { pending.try_send(|state| { assert_eq!(*state, "release"); false }); }
        let mut sent = Vec::new();
        pending.try_send(|state| { sent.push(*state); true });
        pending.try_send(|_| panic!("accepted state must not be replayed"));
        assert_eq!(sent, ["release"]);
    }
}
