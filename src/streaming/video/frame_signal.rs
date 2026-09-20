//! A level-triggered wakeup for the single display consumer. Notifications can coalesce;
//! the pending bit remains set until the newest frame is taken under the output lock.
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

#[derive(Default)]
pub(crate) struct FrameSignal {
    pending: AtomicBool,
    ready: Notify,
}

impl FrameSignal {
    pub(crate) fn is_pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }

    pub(crate) fn set_pending(&self, pending: bool) {
        self.pending.store(pending, Ordering::Release);
    }

    pub(crate) fn wake(&self) {
        self.ready.notify_one();
    }

    pub(crate) async fn wait(&self) {
        while !self.is_pending() {
            // notify_one retains a permit if publication races this registration.
            // An old permit is harmless: always recheck the level after waking.
            self.ready.notified().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn frame_published_before_wait_is_not_lost() {
        let signal = FrameSignal::default();
        signal.set_pending(true);
        signal.wake();
        timeout(Duration::from_millis(100), signal.wait()).await.unwrap();
    }

    #[tokio::test]
    async fn consumed_frame_does_not_trigger_a_phantom_render() {
        let signal = FrameSignal::default();
        signal.set_pending(true);
        signal.wake();
        signal.set_pending(false);
        assert!(timeout(Duration::from_millis(10), signal.wait()).await.is_err());
    }

    #[tokio::test]
    async fn producer_wakes_a_waiting_display_without_a_poll_timer() {
        let signal = Arc::new(FrameSignal::default());
        let waiter_signal = Arc::clone(&signal);
        let waiter = tokio::spawn(async move { waiter_signal.wait().await });
        tokio::task::yield_now().await;
        signal.set_pending(true);
        signal.wake();
        timeout(Duration::from_millis(100), waiter).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn burst_publications_coalesce_without_clearing_the_latest_frame() {
        let signal = FrameSignal::default();
        for _ in 0..100 {
            signal.set_pending(true);
            signal.wake();
        }
        signal.wait().await;
        assert!(signal.is_pending());
        signal.set_pending(false);
        assert!(timeout(Duration::from_millis(10), signal.wait()).await.is_err());
    }
}
