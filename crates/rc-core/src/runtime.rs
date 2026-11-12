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

    /// Runs blocking file or process work on Tokio's blocking pool, off the UI and async worker threads.
    /// Once started, a blocking closure cannot be stopped with `JoinHandle::abort`.
    pub fn spawn_blocking<F, R>(work: F) -> tokio::task::JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        Self::handle().spawn_blocking(work)
    }

    /// Runs a future to completion from a non-async thread (tests, shutdown).
    pub fn block_on<F: Future>(fut: F) -> F::Output {
        Self::handle().block_on(fut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Condvar, Mutex, mpsc};
    use std::time::Duration;

    #[test]
    fn blocking_dotnet_file_reads_do_not_starve_async_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("App.csproj");
        let xml = "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>";
        std::fs::write(&project, xml).unwrap();
        let caller = std::thread::current().id();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (started_tx, started_rx) = mpsc::channel();
        let tasks: Vec<_> = (0..4)
            .map(|_| {
                let project = project.clone();
                let gate = gate.clone();
                let started_tx = started_tx.clone();
                Runtime::spawn_blocking(move || {
                    let contents = std::fs::read_to_string(project).unwrap();
                    started_tx
                        .send((contents, std::thread::current().id()))
                        .unwrap();
                    let (lock, ready) = &*gate;
                    let released = lock.lock().unwrap();
                    drop(ready.wait_while(released, |released| !*released).unwrap());
                })
            })
            .collect();
        for _ in 0..4 {
            let (contents, worker) = started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(contents, xml);
            assert_ne!(worker, caller);
        }

        let quick = Runtime::spawn(async { 42 });
        let result =
            Runtime::block_on(async { tokio::time::timeout(Duration::from_secs(2), quick).await });
        let (lock, ready) = &*gate;
        *lock.lock().unwrap() = true;
        ready.notify_all();
        Runtime::block_on(async {
            for task in tasks {
                task.await.unwrap();
            }
        });
        assert_eq!(result.unwrap().unwrap(), 42);
    }

    #[test]
    fn asynchronous_tasks_can_be_cancelled() {
        let task = Runtime::spawn(std::future::pending::<()>());
        task.abort();
        assert!(Runtime::block_on(task).unwrap_err().is_cancelled());
    }
}
