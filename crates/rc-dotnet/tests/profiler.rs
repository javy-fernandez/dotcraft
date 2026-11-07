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
