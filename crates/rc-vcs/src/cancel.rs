//! Cancellation of running git commands: cancelling kills the git process.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

/// Shared cancellation flag. Clones observe the same flag; `cancel()` kills the git process
/// of every operation that received the token (the operation then fails with `Cancelled`).
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<Inner>);

#[derive(Debug, Default)]
struct Inner {
    cancelled: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    pub fn new() -> CancelToken {
        CancelToken::default()
    }

    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        self.0.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    /// Completes once [`CancelToken::cancel`] has been called.
    pub async fn cancelled(&self) {
        loop {
            let notified = self.0.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}
