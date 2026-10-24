//! The process-wide tokio runtime: language servers, debug adapters, `dotnet` processes and
//! network requests run here, never on the UI thread.

use std::future::Future;
use std::sync::OnceLock;

pub struct Runtime;

impl Runtime {
    /// The shared multi-threaded runtime, started on first use.
    pub fn handle() -> &'static tokio::runtime::Runtime {
        static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        RT.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .thread_name("dotcraft-io")
                .enable_all()
                .build()
                .expect("tokio runtime")
        })
    }

    pub fn spawn<F>(fut: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        Self::handle().spawn(fut)
    }

    /// Runs a future to completion from a non-async thread (tests, shutdown).
    pub fn block_on<F: Future>(fut: F) -> F::Output {
        Self::handle().block_on(fut)
    }
}
