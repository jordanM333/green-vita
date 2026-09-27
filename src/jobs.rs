//! Shared helpers for polling background Tokio tasks without blocking the UI loop.

use anyhow::Result;
use tokio::task::JoinHandle;

pub(crate) enum PollJob<T> {
    Pending(JoinHandle<Result<T>>),
    Done(Result<T>),
}

pub(crate) async fn poll_job<T>(handle: JoinHandle<Result<T>>) -> PollJob<T> {
    if !handle.is_finished() {
        return PollJob::Pending(handle);
    }
    PollJob::Done(
        handle
            .await
            .unwrap_or_else(|error| Err(anyhow::anyhow!("task failed: {error}"))),
    )
}

/// Abort and join so cancellation has actually run before resources are reused.
pub(crate) async fn cancel<T>(handle: JoinHandle<T>) {
    handle.abort();
    let _ = handle.await;
}

#[cfg(test)]
mod cleanup_tests {
    #[tokio::test]
    async fn cancellation_releases_task_owned_resources_before_return() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Lease(Arc<AtomicUsize>);
        impl Drop for Lease {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let active = Arc::new(AtomicUsize::new(0));
        for _ in 0..100 {
            active.fetch_add(1, Ordering::SeqCst);
            let lease = Lease(active.clone());
            let (tx, rx) = tokio::sync::oneshot::channel();
            let job = tokio::spawn(async move {
                let _lease = lease;
                let _ = tx.send(());
                std::future::pending::<()>().await;
            });
            rx.await.unwrap();
            super::cancel(job).await;
            assert_eq!(active.load(Ordering::SeqCst), 0);
        }
    }
}
