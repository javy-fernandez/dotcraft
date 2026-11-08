//! Monitoring (Rider's Monitoring tool window): CPU and memory of the running program, sampled
//! once a second on a thread of its own, plus the .NET runtime's counters (GC heap, collections,
//! thread pool) read through EventPipe when the process exposes its diagnostic port.

use rc_core::Waker;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Samples kept per process: ten minutes.
const MAX_SAMPLES: usize = 600;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sample {
    /// Seconds since monitoring started.
    pub t: f64,
    /// Percent of all the cores (100 = every core busy), as Rider shows it.
    pub cpu: f64,
    /// Working set (resident memory) in MB.
    pub memory_mb: f64,
    /// The runtime's counters of the last interval, when available.
    pub runtime: Option<Runtime>,
}

/// .NET runtime counters (System.Runtime).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Runtime {
    pub gc_heap_mb: Option<f64>,
    pub gen0: Option<f64>,
    pub gen1: Option<f64>,
    pub gen2: Option<f64>,
    pub time_in_gc: Option<f64>,
    /// Bytes allocated in the last interval.
    pub alloc_rate: Option<f64>,
    pub threadpool_threads: Option<f64>,
    pub exceptions: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MonitorTab {
    #[default]
    Charts,
    Memory,
    Cpu,
}

/// The process being watched and what it has reported so far.
pub struct Monitor {
    pub tab: MonitorTab,
    /// Memory snapshot (dotMemory-like): types by size.
    pub snapshot: Option<Result<rc_dotnet::gcdump::HeapSnapshot, String>>,
    snapshot_pending: Option<Receiver<Result<rc_dotnet::gcdump::HeapSnapshot, String>>>,
    /// CPU profile (dotTrace-like): hot methods and call tree.
    pub profile: Option<Result<rc_dotnet::cpu_profile::CpuProfile, String>>,
    profile_pending: Option<Receiver<Result<rc_dotnet::cpu_profile::CpuProfile, String>>>,
    profile_stop: Arc<AtomicBool>,
    pub profile_started: Option<Instant>,
    pub filter: String,
    /// Call tree nodes expanded (paths of names).
    pub expanded: std::collections::HashSet<String>,
    pub show_tree: bool,
    pub name: String,
    pub pid: Option<u32>,
    /// The process monitored once `dotnet watch` and friends start the real program.
    pub app_pid: Option<u32>,
    pub running: bool,
    pub samples: VecDeque<Sample>,
    rx: Option<Receiver<Msg>>,
    stop: Arc<AtomicBool>,
    waker: Waker,
}

enum Msg {
    AppPid(u32),
    Sample(Sample),
    Exited,
}

impl Monitor {
    pub fn new(waker: Waker) -> Self {
        Monitor {
            tab: MonitorTab::Charts,
            snapshot: None,
            snapshot_pending: None,
            profile: None,
            profile_pending: None,
            profile_stop: Arc::new(AtomicBool::new(false)),
            profile_started: None,
            filter: String::new(),
            expanded: std::collections::HashSet::new(),
            show_tree: false,
            name: String::new(), pid: None, app_pid: None, running: false, samples: VecDeque::new(), rx: None, stop: Arc::new(AtomicBool::new(false)), waker }
    }

    /// Follows `target` (the run tab's name and process): starts sampling a new process, and
    /// keeps the last one's charts once it exits.
    pub fn follow(&mut self, target: Option<(String, u32)>) {
        let Some((name, pid)) = target else { return };
        if self.pid == Some(pid) {
            return;
        }
        self.stop.store(true, Ordering::SeqCst);
        self.stop = Arc::new(AtomicBool::new(false));
        self.name = name;
        self.pid = Some(pid);
        self.app_pid = None;
        self.running = true;
        self.samples.clear();
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let stop = self.stop.clone();
        let waker = self.waker.clone();
        let hint = self.name.clone();
        std::thread::Builder::new().name("monitor".into()).spawn(move || sample_loop(pid, hint, stop, tx, waker)).ok();
    }

    /// The process to snapshot or profile (the app, not `dotnet watch`).
    pub fn target_pid(&self) -> Option<u32> {
        self.running.then_some(self.app_pid).flatten()
    }

    pub fn snapshot_running(&self) -> bool {
        self.snapshot_pending.is_some()
    }

    pub fn profiling(&self) -> bool {
        self.profile_pending.is_some()
    }

    /// Get Snapshot: a full GC and the heap's types, on a thread.
    pub fn take_snapshot(&mut self) {
        let Some(pid) = self.target_pid() else { return };
        let (tx, rx) = channel();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let _ = tx.send(rc_dotnet::gcdump::snapshot(pid, Duration::from_secs(60)).map_err(|e| format!("{e:#}")));
            waker.wake();
        });
        self.snapshot_pending = Some(rx);
        self.tab = MonitorTab::Memory;
    }

    /// Start profiling: samples until stopped (or a minute).
    pub fn start_profiling(&mut self) {
        let Some(pid) = self.target_pid() else { return };
        let (tx, rx) = channel();
        let waker = self.waker.clone();
        self.profile_stop = Arc::new(AtomicBool::new(false));
        let stop = self.profile_stop.clone();
        std::thread::spawn(move || {
            let _ = tx.send(rc_dotnet::cpu_profile::profile(pid, Duration::from_secs(60), stop).map_err(|e| format!("{e:#}")));
            waker.wake();
        });
        self.profile_pending = Some(rx);
        self.profile_started = Some(Instant::now());
        self.tab = MonitorTab::Cpu;
    }

    pub fn stop_profiling(&mut self) {
        self.profile_stop.store(true, Ordering::SeqCst);
    }

    pub fn poll(&mut self) {
        if let Some(rx) = &self.snapshot_pending
            && let Ok(r) = rx.try_recv()
        {
            self.snapshot = Some(r);
            self.snapshot_pending = None;
        }
        if let Some(rx) = &self.profile_pending
            && let Ok(r) = rx.try_recv()
        {
            self.expanded.clear();
            self.profile = Some(r);
            self.profile_pending = None;
            self.profile_started = None;
        }
        let Some(rx) = &self.rx else { return };
        for msg in rx.try_iter() {
            match msg {
                Msg::AppPid(p) => {
                    // The launcher's samples (dotnet watch, dotnet run) aren't the app's.
                    self.app_pid = Some(p);
                    self.samples.clear();
                }
                Msg::Sample(s) => {
                    if self.samples.len() == MAX_SAMPLES {
                        self.samples.pop_front();
                    }
                    self.samples.push_back(s);
                }
                Msg::Exited => self.running = false,
            }
        }
    }

    pub fn last(&self) -> Option<&Sample> {
        self.samples.back()
    }

    /// The last runtime counters reported (they may lag a sample behind).
    pub fn runtime(&self) -> Option<&Runtime> {
        self.samples.iter().rev().find_map(|s| s.runtime.as_ref())
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Tools `dotnet watch` and `dotnet run` start besides the app: build nodes, the compiler
/// server, the watcher itself.
fn is_tool(p: &sysinfo::Process) -> bool {
    let cmd: Vec<String> = p.cmd().iter().map(|a| a.to_string_lossy().to_lowercase()).collect();
    let name = p.name().to_string_lossy().to_lowercase();
    // Everything the SDK runs lives in `<dotnet>/sdk/<version>/` (MSBuild, watch, dev-certs,
    // Roslyn, Razor); the app runs from the project's bin folder.
    let from_sdk = cmd.iter().any(|a| a.ends_with(".dll") && (a.contains("/sdk/") || a.contains("\\sdk\\")));
    // `dotnet run`, `dotnet build`…: the CLI; `dotnet App.dll` (or the app's own executable) is the app.
    let cli = (name == "dotnet" || name == "dotnet.exe") && cmd.get(1).is_none_or(|a| !a.ends_with(".dll") && a != "exec");
    from_sdk
        || cli
        || name.contains("vbcscompiler")
        || cmd.iter().any(|a| {
            a.ends_with("msbuild.dll") || a.contains("nodemode") || a.ends_with("dotnet-watch.dll") || a.contains("vbcscompiler") || a.ends_with("buildhost.dll") || a.contains("razor") && a.contains("server")
        })
}

/// The process to measure: among the .NET programs (they have a diagnostic port) below `root`,
/// not the build tools, the one named like the project (`hint`), else the deepest; so
/// `dotnet watch` or `dotnet run` show the app rather than the launcher or MSBuild.
fn app_process(sys: &sysinfo::System, root: u32, hint: &str) -> Option<u32> {
    let hint = hint.split_whitespace().next().unwrap_or("").to_lowercase();
    let mut best: Option<(bool, usize, u32)> = None;
    for (pid, p) in sys.processes() {
        let mut depth = 0;
        let mut cur = p.parent();
        while let Some(parent) = cur {
            depth += 1;
            if parent.as_u32() == root {
                if !is_tool(p) && has_diagnostic_port(pid.as_u32()) {
                    let named = !hint.is_empty() && (p.name().to_string_lossy().to_lowercase().contains(&hint) || p.cmd().iter().any(|a| a.to_string_lossy().to_lowercase().contains(&format!("{hint}.dll"))));
                    let key = (named, depth, pid.as_u32());
                    if best.is_none_or(|b| (key.0, key.1) > (b.0, b.1)) {
                        best = Some(key);
                    }
                }
                break;
            }
            cur = sys.process(parent).and_then(|p| p.parent());
            if depth > 16 {
                break;
            }
        }
    }
    match best {
        Some(b) => Some(b.2),
        // No app below: the root itself, unless it is the launcher (wait for the app).
        None => sys.process(sysinfo::Pid::from_u32(root)).filter(|p| !is_tool(p)).map(|_| root),
    }
}

pub fn has_diagnostic_port(pid: u32) -> bool {
    #[cfg(windows)]
    {
        std::path::Path::new(&format!(r"\\.\pipe\dotnet-diagnostic-{pid}")).exists()
    }
    #[cfg(not(windows))]
    {
        let prefix = format!("dotnet-diagnostic-{pid}-");
        std::fs::read_dir(std::env::temp_dir()).is_ok_and(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with(&prefix)))
    }
}

fn sample_loop(root: u32, hint: String, stop: Arc<AtomicBool>, tx: Sender<Msg>, waker: Waker) {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
    let Ok(mut sys) = System::new_with_specifics(RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing().with_cpu().with_memory().with_cmd(sysinfo::UpdateKind::OnlyIfNotSet))) else {
        let _ = tx.send(Msg::Exited);
        return;
    };
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
    let started = Instant::now();
    let runtime: Arc<Mutex<Option<Runtime>>> = Arc::new(Mutex::new(None));
    // 0 until the app is found (`dotnet watch` first builds, then starts it).
    let mut app = 0;
    // The counters' session of the current app process (`dotnet watch` restarts change it).
    let mut runtime_stop: Option<Arc<AtomicBool>> = None;
    let stop_runtime = |s: &mut Option<Arc<AtomicBool>>| {
        if let Some(s) = s.take() {
            s.store(true, Ordering::SeqCst);
        }
    };
    loop {
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_cpu().with_memory().with_cmd(sysinfo::UpdateKind::OnlyIfNotSet));
        if sys.process(sysinfo::Pid::from_u32(root)).is_none() || stop.load(Ordering::SeqCst) {
            stop_runtime(&mut runtime_stop);
            let _ = tx.send(Msg::Exited);
            waker.wake();
            return;
        }
        let found = app_process(&sys, root, &hint).unwrap_or(0);
        if found != app && found != 0 {
            app = found;
            stop_runtime(&mut runtime_stop);
            let _ = tx.send(Msg::AppPid(app));
        }
        if app != 0 && runtime_stop.is_none() && has_diagnostic_port(app) {
            let s = Arc::new(AtomicBool::new(false));
            start_runtime_counters(app, s.clone(), runtime.clone());
            runtime_stop = Some(s);
        }
        if let Some(p) = sys.process(sysinfo::Pid::from_u32(app)) {
            let sample = Sample {
                t: started.elapsed().as_secs_f64(),
                cpu: (p.cpu_usage() as f64 / cores).clamp(0.0, 100.0),
                memory_mb: p.memory() as f64 / (1024.0 * 1024.0),
                runtime: runtime.lock().ok().and_then(|mut r| r.take()),
            };
            if tx.send(Msg::Sample(sample)).is_err() {
                return;
            }
            waker.wake();
        }
        // sysinfo needs time between refreshes to measure the CPU.
        let next = Instant::now() + Duration::from_secs(1);
        while Instant::now() < next {
            if stop.load(Ordering::SeqCst) {
                stop_runtime(&mut runtime_stop);
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// The runtime's counters, on their own thread; the latest set goes to `latest`.
fn start_runtime_counters(pid: u32, stop: Arc<AtomicBool>, latest: Arc<Mutex<Option<Runtime>>>) {
    std::thread::Builder::new()
        .name("monitor-runtime".into())
        .spawn(move || {
            let _ = rc_dotnet::counters::watch(pid, 1, stop, |c| {
                if let Ok(mut l) = latest.lock() {
                    *l = Some(Runtime {
                        gc_heap_mb: c.gc_heap_mb,
                        gen0: c.gen0_count,
                        gen1: c.gen1_count,
                        gen2: c.gen2_count,
                        time_in_gc: c.time_in_gc_percent,
                        alloc_rate: c.alloc_rate_bytes,
                        threadpool_threads: c.threadpool_threads,
                        exceptions: c.exceptions_per_interval,
                    });
                }
            });
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn samples_a_running_process_and_notices_it_exit() {
        let mut child = std::process::Command::new("sleep").arg("3").spawn().unwrap();
        let mut m = Monitor::new(Waker::noop());
        m.follow(Some(("sleep".into(), child.id())));
        let deadline = Instant::now() + Duration::from_secs(10);
        while m.samples.len() < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            m.poll();
        }
        assert!(m.samples.len() >= 2, "no samples");
        assert!(m.last().unwrap().memory_mb > 0.0);
        let _ = child.wait();
        while m.running && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            m.poll();
        }
        assert!(!m.running);
        assert!(!m.samples.is_empty(), "the charts stay after the exit");
    }
}
