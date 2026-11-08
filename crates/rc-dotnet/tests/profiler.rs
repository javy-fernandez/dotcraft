//! `gcdump::snapshot` and `cpu_profile::profile` against a real .NET process: a console app that
//! keeps `WIDGETS` `Widget` instances alive and (in `cpu` mode) spins in `Work.BusyLoop`.

use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use rc_dotnet::Dotnet;
use rc_dotnet::cpu_profile::{CallNode, profile};
use rc_dotnet::gcdump::snapshot;

const WIDGETS: u64 = 5000;

const PROGRAM: &str = r#"
using System.Runtime.CompilerServices;

var widgets = new List<Widget>();
for (int i = 0; i < 5000; i++) widgets.Add(new Widget(i));
var cpu = args.Length > 0 && args[0] == "cpu";
Console.WriteLine("ready");
var end = DateTime.UtcNow.AddSeconds(60);
double acc = 0;
while (DateTime.UtcNow < end)
{
    if (cpu) acc += Work.BusyLoop(2_000_000);
    else Thread.Sleep(50);
}
GC.KeepAlive(widgets);
Console.WriteLine(acc);

class Widget
{
    public int Id;
    public string Name;
    public Widget(int id) { Id = id; Name = "w" + id; }
}

static class Work
{
    [MethodImpl(MethodImplOptions.NoInlining)]
    public static double BusyLoop(int n)
    {
        double s = 0;
        for (int i = 0; i < n; i++) s += Math.Sqrt(i) * Math.Sin(i);
        return s;
    }
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

/// Builds the app in a temp dir and starts it with `arg`; returns once it printed `ready`.
fn start_app(arg: &str) -> (tempfile::TempDir, Kill) {
    let dotnet = Dotnet::locate(None).expect("the dotnet SDK must be installed");
    let dir = tempfile::Builder::new()
        .prefix("dotcraft-profiler")
        .tempdir()
        .unwrap();
    let root = dunce::canonicalize(dir.path()).unwrap();
    let project = root.join("Prof");
    let ok = dotnet
        .std_command()
        .args(["new", "console", "-n", "Prof", "-o"])
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
        .args(["build", "-c", "Release"])
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
    let mut child = Kill(
        dotnet
            .std_command()
            .arg(out.join("Prof.dll"))
            .arg(arg)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    BufReader::new(child.0.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "ready");
    (dir, child)
}

#[test]
#[ignore = "Requires the .NET SDK"]
fn snapshot_counts_live_widgets() {
    let (_dir, child) = start_app("heap");
    let snap = snapshot(child.0.id(), Duration::from_secs(60)).expect("snapshot failed");
    eprintln!(
        "{} objects, {} bytes, {} types; top: {:#?}",
        snap.total_objects,
        snap.total_bytes,
        snap.types.len(),
        &snap.types[..snap.types.len().min(15)]
    );
    let widget = snap
        .types
        .iter()
        .find(|t| t.name == "Widget")
        .expect("no Widget in the snapshot");
    assert!(
        (WIDGETS..WIDGETS + 10).contains(&widget.count),
        "{widget:?}"
    );
    assert!(widget.size_bytes >= widget.count * 16, "{widget:?}");
    assert!(snap.types.iter().any(|t| t.name == "System.String"));
    assert!(snap.total_objects >= WIDGETS * 2);
    assert!(
        snap.types
            .windows(2)
            .all(|w| w[0].size_bytes >= w[1].size_bytes)
    );
}

#[test]
#[ignore = "Requires the .NET SDK"]
fn profile_finds_busy_loop() {
    let (_dir, child) = start_app("cpu");
    let p = profile(
        child.0.id(),
        Duration::from_secs(3),
        Arc::new(AtomicBool::new(false)),
    )
    .expect("profile failed");
    eprintln!(
        "{} samples; top methods: {:#?}",
        p.samples,
        &p.hot_methods[..p.hot_methods.len().min(10)]
    );
    fn print(n: &CallNode, depth: usize) {
        if depth > 12 || n.total_samples * 50 < 1 {
            return;
        }
        eprintln!(
            "{:indent$}{} ({} / self {})",
            "",
            n.name,
            n.total_samples,
            n.self_samples,
            indent = depth * 2
        );
        for c in n.children.iter().take(3) {
            print(c, depth + 1);
        }
    }
    print(&p.call_tree, 0);
    assert!(p.samples > 100, "{} samples", p.samples);
    let busy = p
        .hot_methods
        .iter()
        .take(5)
        .find(|m| m.name == "Work.BusyLoop")
        .expect("BusyLoop is not among the top methods");
    // The sampler only stops threads at safe points, so the loop is under-represented.
    assert!(busy.total_samples * 5 > p.samples, "{busy:?}");
    assert!(
        !p.hot_methods.iter().any(|m| m.name.contains("PollGC")),
        "suspension frames must be folded"
    );
    assert_eq!(p.call_tree.total_samples, p.samples);
}

/// Copies `from` into `to`, skipping `bin` and `obj`.
fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let name = e.file_name();
        if name == "bin" || name == "obj" {
            continue;
        }
        let path = e.path();
        if path.is_dir() {
            copy_dir(&path, &to.join(&name));
        } else {
            std::fs::copy(&path, to.join(&name)).unwrap();
        }
    }
}

/// One `GET path` on a new connection; `true` on a 200.
fn http_get(port: u16, path: &str) -> bool {
    use std::io::{Read, Write};
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut body = Vec::new();
    let _ = s.read_to_end(&mut body);
    body.starts_with(b"HTTP/1.1 200")
}

/// An ASP.NET Core app (the `Shop.Api` fixture, net8.0) under light HTTP load, with a counters
/// session open and a heap snapshot taken first (as the Monitoring window does), profiled for
/// ~3 s until `stop` is set.
#[test]
#[ignore = "Requires the .NET SDK"]
fn profile_aspnet_app_under_load() {
    use std::sync::atomic::Ordering;

    let dotnet = Dotnet::locate(None).expect("the dotnet SDK must be installed");
    let dir = tempfile::Builder::new()
        .prefix("dotcraft-prof")
        .tempdir()
        .unwrap();
    let root = dunce::canonicalize(dir.path()).unwrap();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src");
    for p in ["Shop.Api", "Shop.Core"] {
        copy_dir(&fixture.join(p), &root.join(p));
    }
    let project = root.join("Shop.Api");
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
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    // `DOTCRAFT_TEST_DOTNET_WATCH=1` runs the app under `dotnet watch` as the IDE does (the
    // profiled pid is then the app's, found by its listening port with `lsof`; Unix only).
    let watch = std::env::var_os("DOTCRAFT_TEST_DOTNET_WATCH").is_some();
    let child = Kill(if watch {
        dotnet
            .std_command()
            .arg("watch")
            .arg("--project")
            .arg(&project)
            .args([
                "run",
                "--no-launch-profile",
                "--urls",
                &format!("http://127.0.0.1:{port}"),
            ])
            .current_dir(&project)
            .stdout(Stdio::null())
            .spawn()
            .unwrap()
    } else {
        dotnet
            .std_command()
            .arg(out.join("Shop.Api.dll"))
            .args(["--urls", &format!("http://127.0.0.1:{port}")])
            .current_dir(&project)
            .stdout(Stdio::null())
            .spawn()
            .unwrap()
    });
    let started = std::time::Instant::now();
    while !http_get(port, "/health") {
        assert!(
            started.elapsed() < Duration::from_secs(90),
            "the app did not start"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    let pid = if watch {
        let o = std::process::Command::new("lsof")
            .args(["-ti", &format!("tcp:{port}"), "-sTCP:LISTEN"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout)
            .trim()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap()
    } else {
        child.0.id()
    };
    eprintln!("profiling pid {pid}");
    // As the IDE's Monitoring window does: a counters session stays open on the same process,
    // and a heap snapshot may have been taken first.
    let counters_stop = Arc::new(AtomicBool::new(false));
    let counters = {
        let stop = counters_stop.clone();
        std::thread::spawn(move || rc_dotnet::counters::watch(pid, 1, stop, |_| {}))
    };
    let heap = snapshot(pid, Duration::from_secs(60)).expect("snapshot failed");
    assert!(
        heap.types
            .iter()
            .any(|t| t.name.starts_with("Microsoft.AspNetCore"))
    );

    let done = Arc::new(AtomicBool::new(false));
    let load: Vec<_> = (0..4)
        .map(|_| {
            let done = done.clone();
            std::thread::spawn(move || {
                let mut ok = 0u64;
                while !done.load(Ordering::Relaxed) {
                    ok += u64::from(http_get(port, "/weatherforecast"));
                    // Light load, like a `curl` loop: most of the time the app is idle.
                    std::thread::sleep(Duration::from_millis(20));
                }
                ok
            })
        })
        .collect();
    let stop = Arc::new(AtomicBool::new(false));
    let stopper = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(3));
            stop.store(true, Ordering::Relaxed);
        })
    };
    let started = std::time::Instant::now();
    let p = profile(pid, Duration::from_secs(60), stop).expect("profile failed");
    let took = started.elapsed();
    done.store(true, Ordering::Relaxed);
    stopper.join().unwrap();
    counters_stop.store(true, Ordering::Relaxed);
    counters.join().unwrap().expect("counters failed");
    let requests: u64 = load.into_iter().map(|t| t.join().unwrap()).sum();
    eprintln!(
        "{requests} requests; profile took {took:?}; {} samples ({} idle); top methods: {:#?}",
        p.samples,
        p.idle_samples,
        &p.hot_methods[..p.hot_methods.len().min(15)]
    );
    assert!(requests > 0, "no request succeeded");
    assert!(
        took < Duration::from_secs(25),
        "stop did not end the profile"
    );
    assert!(p.samples > 0, "no samples");
    assert!(
        p.hot_methods
            .iter()
            .any(|m| m.name.starts_with("Microsoft.AspNetCore") || m.name.starts_with("Shop.Api")),
        "no ASP.NET Core / Shop.Api method among the hot methods"
    );
    assert!(
        !p.hot_methods
            .iter()
            .any(|m| m.name.contains("FileSystemWatcher")),
        "idle threads must be left out"
    );
}
