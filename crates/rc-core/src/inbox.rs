//! Background work reports to the UI through inboxes: a channel plus a waker that asks the
//! UI for a new frame (`ctx.request_repaint`), so the UI never polls or blocks.

use crossbeam_channel::{Receiver, Sender, unbounded};
use std::sync::Arc;

/// Asks the UI thread to repaint (wakes the event loop).
#[derive(Clone)]
pub struct Waker(Arc<dyn Fn() + Send + Sync>);

impl Waker {
    pub fn new(f: impl Fn() + Send + Sync + 'static) -> Self {
        Waker(Arc::new(f))
    }

    /// A waker that does nothing (tests, headless runs).
    pub fn noop() -> Self {
        Waker(Arc::new(|| {}))
    }

    pub fn wake(&self) {
        (self.0)()
    }

    pub fn as_fn(&self) -> Arc<dyn Fn() + Send + Sync> {
        self.0.clone()
    }
}

impl std::fmt::Debug for Waker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Waker")
    }
}

/// Receiving end kept by the UI; [`Inbox::sender`] hands out senders that wake it.
pub struct Inbox<T> {
    tx: Sender<T>,
    rx: Receiver<T>,
    waker: Waker,
}

/// A sender that wakes the UI after every message.
pub struct Outbox<T> {
    tx: Sender<T>,
    waker: Waker,
}

impl<T> Clone for Outbox<T> {
    fn clone(&self) -> Self {
        Outbox { tx: self.tx.clone(), waker: self.waker.clone() }
    }
}

impl<T> Outbox<T> {
    /// Sends and wakes the UI; false when the inbox is gone.
    pub fn send(&self, msg: T) -> bool {
        let ok = self.tx.send(msg).is_ok();
        self.waker.wake();
        ok
    }
}

impl<T> Inbox<T> {
    pub fn new(waker: Waker) -> Self {
        let (tx, rx) = unbounded();
        Inbox { tx, rx, waker }
    }

    pub fn sender(&self) -> Outbox<T> {
        Outbox { tx: self.tx.clone(), waker: self.waker.clone() }
    }

    /// Everything received so far, without blocking.
    pub fn drain(&self) -> Vec<T> {
        self.rx.try_iter().collect()
    }

    /// Waits for one message (tests).
    pub fn recv_timeout(&self, t: std::time::Duration) -> Option<T> {
        self.rx.recv_timeout(t).ok()
    }
}
