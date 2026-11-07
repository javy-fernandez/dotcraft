//! Sampling CPU profiler for a running .NET process (a light `dotnet-trace` + call tree), without
//! external tools.
//!
//! [`profile`] starts an EventPipe session with `Microsoft-DotNETCore-SampleProfiler` (the runtime
//! suspends the managed threads every ~1 ms and logs one `ThreadSample` with a stack per thread)
//! and `Microsoft-Windows-DotNETRuntime` with the JIT/Loader keywords (`MethodLoadVerbose` for
//! methods compiled during the session), and requests a rundown so that, when the session ends,
//! the runtime lists every method that already had code (`MethodDCEndVerbose`). Instruction
//! pointers are resolved to `Namespace.Type.Method` with those code ranges; the rest is shown as
//! `[native]`.
//!
//! Only on-CPU samples count. Samples of threads running managed code (`ThreadSample` type 2)
//! always do; "external" samples (type 1: waiting or in native code) count unless the thread is
//! blocked in a known wait (`Thread.Sleep`, `WaitHandle.Wait*`, `Monitor.Wait`, socket polls...) or
//! has no managed frame at all. The type alone is not enough: some runtimes (.NET 10 on macOS
//! arm64) report every sample as external, busy threads included. Frames of `Thread.PollGC`
//! (where a thread parks while the sampler suspends it) are folded into their caller.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::eventpipe::{
    Cursor, EventHeader, EventMeta, Provider, Sink, collect_tracing2_payload, run_session,
};

/// Samples spent in one method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodStats {
    /// `Namespace.Type.Method`, or `[native]`.
    pub name: String,
    /// Samples with the method at the top of the stack.
    pub self_samples: u64,
    /// Samples with the method anywhere on the stack (recursion counted once).
    pub total_samples: u64,
}

/// One node of the call tree: a method called along one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallNode {
    pub name: String,
    /// Samples in this node and its callees.
    pub total_samples: u64,
    /// Samples with this node at the top of the stack.
    pub self_samples: u64,
    /// Callees, most samples first.
    pub children: Vec<CallNode>,
}

/// The result of a CPU profiling session.
#[derive(Debug, Clone, PartialEq)]
pub struct CpuProfile {
    /// On-CPU samples counted (waits excluded); the runtime samples every ~1 ms.
    pub samples: u64,
    /// Up to [`TOP_METHODS`] methods, by `total_samples` descending.
    pub hot_methods: Vec<MethodStats>,
    /// `All threads` > `Thread <id>` > outermost frame > ... > leaf frame.
    pub call_tree: CallNode,
}

/// Maximum number of [`CpuProfile::hot_methods`].
pub const TOP_METHODS: usize = 200;
/// Name of the frames that are not managed code of a known method.
pub const NATIVE_FRAME: &str = "[native]";

const SAMPLE_PROVIDER: &str = "Microsoft-DotNETCore-SampleProfiler";
const RUNTIME_PROVIDER: &str = "Microsoft-Windows-DotNETRuntime";
const RUNDOWN_PROVIDER: &str = "Microsoft-Windows-DotNETRuntimeRundown";
const KEYWORD_LOADER: u64 = 0x8;
const KEYWORD_JIT: u64 = 0x10;
const KEYWORD_NGEN: u64 = 0x20;

/// `ThreadSample` (SampleProfiler provider).
const EVENT_THREAD_SAMPLE: i32 = 0;
/// `MethodLoadVerbose` (runtime) / `MethodDCStartVerbose` (rundown).
const EVENT_METHOD_VERBOSE_A: i32 = 143;
/// `MethodUnloadVerbose` (runtime) / `MethodDCEndVerbose` (rundown).
const EVENT_METHOD_VERBOSE_B: i32 = 144;
/// `ThreadSample.Type` of a thread running managed code.
const SAMPLE_MANAGED: u32 = 2;

/// Samples the CPU of `pid` for `duration` (or until `stop` is set, which ends the session early
/// and still returns what was collected) and builds the hot methods and the call tree.
///
/// Blocking; run it on its own thread. After the sampling it waits (up to 15 s) for the method
/// rundown that names precompiled and already-jitted methods. Waits up to 10 s for the diagnostic
/// port to appear. Errors when the port cannot be reached, the runtime rejects the session, or
/// the stream is malformed. If the process exits meanwhile, the samples so far are returned.
pub fn profile(pid: u32, duration: Duration, stop: Arc<AtomicBool>) -> Result<CpuProfile> {
    let payload = collect_tracing2_payload(
        256,
        true,
        &[
            Provider {
                name: SAMPLE_PROVIDER,
                keywords: 0,
                level: 4,
                args: None,
            },
            Provider {
                name: RUNTIME_PROVIDER,
                keywords: KEYWORD_LOADER | KEYWORD_JIT | KEYWORD_NGEN,
                level: 5,
                args: None,
            },
        ],
    );
    let end = AtomicBool::new(false);
    let mut samples = SampleCollector::default();
    run_session(
        pid,
        &payload,
        &stop,
        &end,
        Some(Instant::now() + duration),
        Duration::from_secs(15),
        &mut samples,
    )?;
    Ok(samples.into_profile())
}

/// Collects samples, stacks and method code ranges of a session.
#[derive(Default)]
struct SampleCollector {
    pointer_size: usize,
    /// Stack id -> index in `unique_stacks` (ids may be reused after a sequence point).
    stack_ids: HashMap<u32, usize>,
    unique_stacks: Vec<Vec<u64>>,
    stack_index: HashMap<Vec<u64>, usize>,
    /// `(thread, stack index or usize::MAX for none, managed sample)` -> samples.
    counts: HashMap<(u64, usize, bool), u64>,
    /// `(start, size, name)` of every method with code.
    methods: Vec<(u64, u64, String)>,
}

impl SampleCollector {
    /// `MethodLoadVerbose` / `MethodDCEndVerbose`: `MethodID u64, ModuleID u64,
    /// MethodStartAddress u64, MethodSize u32, MethodToken u32, MethodFlags u32,
    /// MethodNamespace utf16z, MethodName utf16z, MethodSignature utf16z, ...`.
    fn method(&mut self, payload: &[u8]) -> Result<()> {
        let mut c = Cursor::new(payload);
        let _id = c.u64()?;
        let _module = c.u64()?;
        let start = c.u64()?;
        let size = c.u32()?;
        let _token = c.u32()?;
        let _flags = c.u32()?;
        let namespace = c.utf16z()?;
        let name = c.utf16z()?;
        if size == 0 {
            return Ok(());
        }
        let full = if namespace.is_empty() {
            name
        } else {
            format!("{namespace}.{name}")
        };
        self.methods.push((start, u64::from(size), full));
        Ok(())
    }

    fn into_profile(mut self) -> CpuProfile {
        self.methods.sort_by_key(|m| m.0);
        let methods = self.methods;
        let mut names: HashMap<(u64, bool), String> = HashMap::new();
        let mut resolve = |ip: u64, leaf: bool| -> String {
            names
                .entry((ip, leaf))
                .or_insert_with(|| {
                    // Callers' IPs are return addresses: look up the call instruction instead.
                    let at = if leaf { ip } else { ip.saturating_sub(1) };
                    let i = methods.partition_point(|m| m.0 <= at);
                    match i.checked_sub(1).map(|i| &methods[i]) {
                        Some((start, size, name)) if at < start + size => name.clone(),
                        _ => NATIVE_FRAME.to_string(),
                    }
                })
                .clone()
        };
        let mut stacks: Vec<(u64, Vec<String>, u64)> = Vec::new();
        for (&(thread, stack, managed), &n) in &self.counts {
            let mut frames = match self.unique_stacks.get(stack) {
                Some(ips) => frame_names(ips, &mut resolve),
                None => Vec::new(),
            };
            while frames.last().is_some_and(|f| is_suspension_frame(f)) {
                frames.pop();
            }
            if !managed && is_idle(&frames) {
                continue;
            }
            stacks.push((thread, frames, n));
        }
        stacks.sort();
        build_profile(&stacks)
    }
}

/// Frames where a thread parks while the sampler suspends it: the sample belongs to the caller.
fn is_suspension_frame(name: &str) -> bool {
    name.starts_with("System.Threading.Thread.PollGC")
        || name.starts_with("System.Threading.Thread.<PollGC>")
}

/// Managed methods that block the thread (waits, sleeps, joins, I/O polls).
const WAIT_METHODS: &[&str] = &[
    "System.Threading.Thread.Sleep",
    "System.Threading.Thread.Join",
    "System.Threading.Monitor.Wait",
    "System.Threading.Monitor.ObjWait",
    "System.Threading.WaitHandle.Wait",
    "System.Threading.WaitHandle.SignalAndWait",
    "System.Threading.LowLevelLifoSemaphore.Wait",
    "System.Threading.LowLevelMonitor.Wait",
    "System.Threading.WaitSubsystem.",
    "System.Threading.WaitSubsystem+",
    "Interop+Sys.WaitForSocketEvents",
    "Interop+Sys.Poll",
    "Interop+Sys.Read",
    "Interop+Kernel32.GetQueuedCompletionStatus",
    "Interop+Kernel32.ReadFile",
];

/// Whether a non-managed ("external") sample is a thread waiting rather than using the CPU:
/// its innermost managed frame is a blocking method, or it has no managed frame at all (runtime
/// threads such as the finalizer waiting in native code).
///
/// Needed because some runtimes (seen on .NET 10 / macOS arm64) report every sample as
/// external, busy threads included.
fn is_idle(frames: &[String]) -> bool {
    match frames.iter().rev().find(|f| *f != NATIVE_FRAME) {
        None => true,
        Some(f) => WAIT_METHODS.iter().any(|w| f.starts_with(w)),
    }
}

/// Root-first frame names of a leaf-first IP stack, consecutive `[native]` frames merged.
fn frame_names(ips: &[u64], resolve: &mut impl FnMut(u64, bool) -> String) -> Vec<String> {
    let mut frames: Vec<String> = Vec::with_capacity(ips.len());
    for (i, &ip) in ips.iter().enumerate().rev() {
        let name = resolve(ip, i == 0);
        if name == NATIVE_FRAME && frames.last().is_some_and(|f| f == NATIVE_FRAME) {
            continue;
        }
        frames.push(name);
    }
    frames
}

/// Builds the profile from `(thread, root-first frames, samples)`.
fn build_profile(stacks: &[(u64, Vec<String>, u64)]) -> CpuProfile {
    struct Node {
        name: String,
        total: u64,
        self_: u64,
        children: HashMap<String, usize>,
    }
    let new = |name: String| Node {
        name,
        total: 0,
        self_: 0,
        children: HashMap::new(),
    };
    let mut nodes = vec![new("All threads".into())];
    let child = |nodes: &mut Vec<Node>, parent: usize, name: &str| -> usize {
        if let Some(&i) = nodes[parent].children.get(name) {
            return i;
        }
        nodes.push(new(name.to_string()));
        let i = nodes.len() - 1;
        nodes[parent].children.insert(name.to_string(), i);
        i
    };
    let mut methods: HashMap<&str, (u64, u64)> = HashMap::new();
    let mut total = 0;
    for (thread, frames, n) in stacks {
        total += n;
        nodes[0].total += n;
        let mut at = child(&mut nodes, 0, &format!("Thread {thread}"));
        nodes[at].total += n;
        for f in frames {
            at = child(&mut nodes, at, f);
            nodes[at].total += n;
        }
        nodes[at].self_ += n;
        if let Some(leaf) = frames.last() {
            methods.entry(leaf).or_default().0 += n;
        }
        let mut seen: Vec<&str> = Vec::new();
        for f in frames {
            if !seen.contains(&f.as_str()) {
                seen.push(f);
                methods.entry(f).or_default().1 += n;
            }
        }
    }

    fn finish(nodes: &mut [Node], i: usize) -> CallNode {
        let mut kids: Vec<usize> = nodes[i].children.values().copied().collect();
        kids.sort_unstable();
        let mut children: Vec<CallNode> = kids.into_iter().map(|k| finish(nodes, k)).collect();
        children.sort_by(|a, b| {
            b.total_samples
                .cmp(&a.total_samples)
                .then_with(|| a.name.cmp(&b.name))
        });
        CallNode {
            name: std::mem::take(&mut nodes[i].name),
            total_samples: nodes[i].total,
            self_samples: nodes[i].self_,
            children,
        }
    }

    let mut hot_methods: Vec<MethodStats> = methods
        .into_iter()
        .map(|(name, (self_samples, total_samples))| MethodStats {
            name: name.to_string(),
            self_samples,
            total_samples,
        })
        .collect();
    hot_methods.sort_by(|a, b| {
        b.total_samples
            .cmp(&a.total_samples)
            .then_with(|| b.self_samples.cmp(&a.self_samples))
            .then_with(|| a.name.cmp(&b.name))
    });
    hot_methods.truncate(TOP_METHODS);
    CpuProfile {
        samples: total,
        hot_methods,
        call_tree: finish(&mut nodes, 0),
    }
}

impl Sink for SampleCollector {
    fn trace(&mut self, pointer_size: usize) {
        self.pointer_size = pointer_size;
    }

    fn stack(&mut self, id: u32, ips: &[u64]) {
        let index = match self.stack_index.get(ips) {
            Some(&i) => i,
            None => {
                self.unique_stacks.push(ips.to_vec());
                let i = self.unique_stacks.len() - 1;
                self.stack_index.insert(ips.to_vec(), i);
                i
            }
        };
        self.stack_ids.insert(id, index);
    }

    fn event(&mut self, meta: &EventMeta, header: &EventHeader, payload: &[u8]) -> Result<()> {
        match (meta.provider.as_str(), meta.event_id) {
            (SAMPLE_PROVIDER, EVENT_THREAD_SAMPLE) => {
                let managed =
                    Cursor::new(payload).u32().unwrap_or(SAMPLE_MANAGED) == SAMPLE_MANAGED;
                let stack = match header.stack {
                    0 => usize::MAX,
                    id => self.stack_ids.get(&id).copied().unwrap_or(usize::MAX),
                };
                *self
                    .counts
                    .entry((header.thread, stack, managed))
                    .or_default() += 1;
            }
            (
                RUNTIME_PROVIDER | RUNDOWN_PROVIDER,
                EVENT_METHOD_VERBOSE_A | EVENT_METHOD_VERBOSE_B,
            ) => {
                // A malformed method event only loses that method's name.
                let _ = self.method(payload);
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eventpipe::parse_nettrace;
    use crate::eventpipe::test_support::*;

    fn method_event(start: u64, size: u32, namespace: &str, name: &str) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&1u64.to_le_bytes());
        p.extend_from_slice(&2u64.to_le_bytes());
        p.extend_from_slice(&start.to_le_bytes());
        p.extend_from_slice(&size.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        utf16z(&mut p, namespace);
        utf16z(&mut p, name);
        utf16z(&mut p, "void  ()");
        p.extend_from_slice(&0u16.to_le_bytes());
        p
    }

    fn sample(thread: u64, stack: u32, kind: u32) -> Blob {
        Blob {
            metadata_id: 1,
            thread,
            stack,
            payload: kind.to_le_bytes().to_vec(),
        }
    }

    fn parse(blocks: &[(&str, Vec<u8>)]) -> CpuProfile {
        let mut c = SampleCollector::default();
        parse_nettrace(stream(blocks).as_slice(), &mut c).unwrap();
        c.into_profile()
    }

    #[test]
    fn builds_hot_methods_and_call_tree() {
        let meta = compressed_blobs(&[
            (
                0,
                metadata_with_event_id(1, SAMPLE_PROVIDER, 0, "ThreadSample"),
            ),
            (0, metadata_with_event_id(2, RUNTIME_PROVIDER, 143, "")),
            (0, metadata_with_event_id(3, RUNDOWN_PROVIDER, 144, "")),
        ]);
        // Main @0x1000..0x1100, Work.BusyLoop @0x2000..0x2100, Math.Sqrt @0x3000..0x3010.
        let stacks = stack_block(
            1,
            &[
                // leaf first: BusyLoop <- Main
                &[0x2010, 0x1050],
                // Sqrt <- BusyLoop <- native <- native <- Main
                &[0x3004, 0x2020, 0x9999, 0x8888, 0x1060],
                // BusyLoop <- Main, whose return address is the end of Main's range
                &[0x2050, 0x1100],
                // native <- Thread.Sleep <- Main: waiting
                &[0x9000, 0x4010, 0x1070],
                // PollGC <- BusyLoop <- Main: suspended in BusyLoop
                &[0x5004, 0x2030, 0x1040],
                // native only
                &[0x9000],
            ],
        );
        let events = compressed_events(&[
            sample(7, 1, 2),
            sample(7, 1, 2),
            sample(7, 2, 2),
            sample(7, 3, 2),
            sample(8, 4, 1),  // external, waiting: ignored
            sample(11, 6, 1), // external, no managed frame: ignored
            sample(11, 0, 1), // external, no stack: ignored
            sample(10, 5, 1), // external but busy: counted
            sample(9, 1, 2),
            Blob {
                metadata_id: 2,
                thread: 7,
                stack: 0,
                payload: method_event(0x2000, 0x100, "App.Work", "BusyLoop"),
            },
        ]);
        let rundown = compressed_blobs(&[
            (3, method_event(0x1000, 0x100, "Program", "Main")),
            (3, method_event(0x3000, 0x10, "System.Math", "Sqrt")),
            (
                3,
                method_event(0x4000, 0x20, "System.Threading.Thread", "Sleep"),
            ),
            (
                3,
                method_event(
                    0x5000,
                    0x20,
                    "System.Threading.Thread",
                    "<PollGC>g__PollGCWorker|67_0",
                ),
            ),
        ]);
        let p = parse(&[
            ("MetadataBlock", meta),
            ("StackBlock", stacks),
            ("EventBlock", events),
            ("EventBlock", rundown),
        ]);
        assert_eq!(p.samples, 6);
        let hot: Vec<(&str, u64, u64)> = p
            .hot_methods
            .iter()
            .map(|m| (m.name.as_str(), m.self_samples, m.total_samples))
            .collect();
        assert_eq!(
            hot,
            [
                ("App.Work.BusyLoop", 5, 6),
                ("Program.Main", 0, 6),
                ("System.Math.Sqrt", 1, 1),
                ("[native]", 0, 1),
            ]
        );
        let root = &p.call_tree;
        assert_eq!((root.name.as_str(), root.total_samples), ("All threads", 6));
        let t7 = &root.children[0];
        assert_eq!((t7.name.as_str(), t7.total_samples), ("Thread 7", 4));
        let main = &t7.children[0];
        assert_eq!(
            (main.name.as_str(), main.total_samples, main.self_samples),
            ("Program.Main", 4, 0)
        );
        assert_eq!(main.children[0].total_samples, 3);
        let names: Vec<&str> = main.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["App.Work.BusyLoop", "[native]"]);
        let native = &main.children[1];
        assert_eq!(native.children[0].name, "App.Work.BusyLoop");
        assert_eq!(native.children[0].children[0].name, "System.Math.Sqrt");
        let threads: Vec<&str> = root.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(threads, ["Thread 7", "Thread 10", "Thread 9"]);
        let t10 = &root.children[1].children[0].children[0];
        assert_eq!(
            (t10.name.as_str(), t10.self_samples),
            ("App.Work.BusyLoop", 1)
        );
    }

    #[test]
    fn samples_without_stack_count_on_the_thread() {
        let meta = compressed_blobs(&[(0, metadata_with_event_id(1, SAMPLE_PROVIDER, 0, ""))]);
        let p = parse(&[
            ("MetadataBlock", meta),
            (
                "EventBlock",
                compressed_events(&[sample(3, 0, 2), sample(3, 42, 2)]),
            ),
        ]);
        assert_eq!(p.samples, 2);
        assert!(p.hot_methods.is_empty());
        let t = &p.call_tree.children[0];
        assert_eq!((t.total_samples, t.self_samples), (2, 2));
    }

    #[test]
    fn hot_methods_are_capped() {
        let stacks: Vec<(u64, Vec<String>, u64)> = (0..300)
            .map(|i| (1, vec![format!("M{i}")], 1 + i as u64))
            .collect();
        let p = build_profile(&stacks);
        assert_eq!(p.hot_methods.len(), TOP_METHODS);
        assert_eq!(p.hot_methods[0].name, "M299");
    }
}
