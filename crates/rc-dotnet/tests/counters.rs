//! `counters::watch` against a real .NET process: a console app that allocates in a loop.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rc_dotnet::Dotnet;
use rc_dotnet::counters::{RuntimeCounters, watch};

const PROGRAM: &str = r#"
var list = new System.Collections.Generic.List<byte[]>();
var end = DateTime.UtcNow.AddSeconds(30);
while (DateTime.UtcNow < end)
{
    list.Add(new byte[16 * 1024]);
    if (list.Count > 2000) list.Clear();
    if (list.Count % 100 == 0) Thread.Sleep(1);
}
"#;

/// Kills the app when the test ends, even on a failed assertion.
struct Kill(std::process::Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "Requires the .NET SDK"]
fn reads_runtime_counters_of_a_running_app() {
    let dotnet = Dotnet::locate(None).expect("the dotnet SDK must be installed");
    let dir = tempfile::Builder::new()
        .prefix("dotcraft-counters")
        .tempdir()
        .unwrap();
    let root = dunce::canonicalize(dir.path()).unwrap();
    let project = root.join("Alloc");
    let ok = dotnet
        .std_command()
        .args(["new", "console", "-n", "Alloc", "-o"])
        .arg(&project)
        .output()
        .unwrap();
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stdout)
    );
    std::fs::write(project.join("Program.cs"), PROGRAM).unwrap();
    let out = project.join("out");
    let ok = dotnet
        .std_command()
        .arg("build")
        .arg(&project)
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stdout)
    );

    let child = Kill(
        dotnet
            .std_command()
            .arg(out.join("Alloc.dll"))
            .spawn()
            .unwrap(),
    );
    let pid = child.0.id();
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<RuntimeCounters>();
    let watcher = {
        let stop = stop.clone();
        std::thread::spawn(move || watch(pid, 1, stop, move |c| drop(tx.send(c))))
    };

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut samples = Vec::new();
    let found = loop {
        if let Ok(c) = rx.recv_timeout(Duration::from_millis(200)) {
            let ok = c.gc_heap_mb.is_some() && c.gen0_count.is_some();
            samples.push(c);
            if ok && samples.len() >= 2 {
                break true;
            }
        }
        if Instant::now() > deadline || watcher.is_finished() {
            break false;
        }
    };
    let last = samples.last().cloned();
    eprintln!(
        "{} samples, sizes {:?}; last: {last:#?}",
        samples.len(),
        samples.iter().map(|s| s.values.len()).collect::<Vec<_>>()
    );

    stop.store(true, Ordering::Relaxed);
    let started = Instant::now();
    while !watcher.is_finished() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "watch did not return after stop"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    watcher.join().unwrap().expect("watch failed");
    assert!(
        found,
        "no sample with gc-heap-size and gen-0-gc-count: {samples:#?}"
    );
    let last = last.unwrap();
    assert!(last.gc_heap_mb.unwrap() > 0.0);
    assert!(last.alloc_rate_bytes.is_some_and(|b| b > 0.0), "{last:#?}");
}
