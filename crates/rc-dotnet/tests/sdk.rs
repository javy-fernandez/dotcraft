//! Integration tests against the real `dotnet` SDK and `tests/fixtures/Shop`.
//! Tests that build inside the fixture take a lock (MSBuild does not like two builds writing
//! the same `obj/`); tests that break or modify code work on a temporary copy.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::build::{BuildAction, BuildEvent, BuildRequest, start_build};
use rc_dotnet::run::{RunEvent, RunRequest, start_run};
use rc_dotnet::test::{TestEvent, TestFilter, TestRequest, discover_tests, start_tests};
use rc_dotnet::{CancelToken, Dotnet, Severity, Stream, TestOutcome, msbuild, nuget};

static FIXTURE_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    FIXTURE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn shop() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop").canonicalize().unwrap()
}

fn dotnet() -> Dotnet {
    Dotnet::locate(None).expect("the dotnet SDK must be installed")
}

/// Receives events until `last` matches one (returned last) or the timeout expires.
fn collect<T>(inbox: &Inbox<T>, timeout: Duration, last: impl Fn(&T) -> bool) -> Vec<T> {
    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();
    while Instant::now() < deadline {
        if let Some(e) = inbox.recv_timeout(Duration::from_millis(200)) {
            let done = last(&e);
            out.push(e);
            if done {
                return out;
            }
        }
    }
    panic!("no final event within {timeout:?}");
}

/// Copies a folder without `bin/`, `obj/` and `TestResults/`.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let name = e.file_name();
        if ["bin", "obj", "TestResults"].iter().any(|n| name == *n) {
            continue;
        }
        let target = to.join(&name);
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

fn temp_copy() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new().prefix("dotcraft-shop-").tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap().join("Shop");
    copy_tree(&shop(), &root);
    (dir, root)
}

fn build_finished(e: &BuildEvent) -> bool {
    matches!(e, BuildEvent::Finished(_) | BuildEvent::Failed(_))
}

fn build_result(events: &[BuildEvent]) -> rc_dotnet::BuildResult {
    match events.last() {
        Some(BuildEvent::Finished(r)) => r.clone(),
        other => panic!("build did not finish: {other:?}"),
    }
}

#[test]
fn locates_the_sdk_and_lists_sdks_and_runtimes() {
    let d = dotnet();
    assert!(d.path.is_file());
    let sdks = d.list_sdks().unwrap();
    assert!(sdks.iter().any(|s| s.major() == 8), "{sdks:?}");
    assert!(sdks.iter().any(|s| s.major() == 10), "{sdks:?}");
    let runtimes = d.list_runtimes().unwrap();
    assert!(runtimes.iter().any(|r| r.name == "Microsoft.NETCore.App"));
    assert!(runtimes.iter().any(|r| r.name == "Microsoft.AspNetCore.App"));
    assert!(!d.version(Some(&shop())).unwrap().is_empty());
    // An explicit folder is accepted too.
    assert_eq!(Dotnet::locate(d.root()).map(|x| x.path), Some(d.path.clone()));
}

#[test]
fn evaluates_msbuild_properties() {
    let d = dotnet();
    Runtime::block_on(async {
        let cli = shop().join("src/Shop.Cli/Shop.Cli.csproj");
        let p = msbuild::evaluate(&d, &cli, "Debug", None).await.unwrap();
        assert_eq!(p.target_framework, "net8.0");
        assert_eq!(p.target_frameworks, vec!["net8.0"]);
        assert!(p.is_executable() && !p.is_web && !p.is_test_project);
        assert_eq!(p.assembly_name, "Shop.Cli");
        assert_eq!(p.root_namespace, "Shop.Cli");
        assert!(p.target_path.ends_with("bin/Debug/net8.0/Shop.Cli.dll"), "{:?}", p.target_path);
        assert_eq!(p.project_dir.canonicalize().unwrap(), shop().join("src/Shop.Cli"));

        let api = msbuild::evaluate(&d, &shop().join("src/Shop.Api/Shop.Api.csproj"), "Release", None).await.unwrap();
        assert!(api.is_web);
        assert!(api.target_path.ends_with("bin/Release/net8.0/Shop.Api.dll"));

        let one = msbuild::get_properties(&d, &cli, &["OutputType"], &[]).await.unwrap();
        assert_eq!(one["OutputType"], "Exe");

        // Multi-targeting: the outer evaluation has no TargetPath; evaluate() asks the inner one.
        let dir = tempfile::tempdir().unwrap();
        let multi = dir.path().join("Multi.csproj");
        std::fs::write(&multi, r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFrameworks>net8.0;net10.0</TargetFrameworks></PropertyGroup></Project>"#).unwrap();
        let m = msbuild::evaluate(&d, &multi, "Debug", None).await.unwrap();
        assert_eq!(m.target_frameworks, vec!["net8.0", "net10.0"]);
        assert_eq!(m.target_framework, "net8.0");
        assert!(m.target_path.ends_with("net8.0/Multi.dll"), "{:?}", m.target_path);
        let m10 = msbuild::evaluate(&d, &multi, "Debug", Some("net10.0")).await.unwrap();
        assert!(m10.target_path.ends_with("net10.0/Multi.dll"));
    });
}

#[test]
fn builds_the_solution_with_no_errors() {
    let _g = lock();
    let inbox = Inbox::new(Waker::noop());
    let _h = start_build(&dotnet(), BuildRequest::new(shop().join("Shop.sln"), BuildAction::Build), inbox.sender());
    let events = collect(&inbox, Duration::from_secs(600), build_finished);
    let r = build_result(&events);
    assert!(r.success, "{r:?}");
    assert_eq!(r.errors, 0);
    assert!(matches!(&events[0], BuildEvent::Started { command } if command.contains("build")));
    let outputs: Vec<&str> = events.iter().filter_map(|e| if let BuildEvent::Output(l) = e { Some(l.text.as_str()) } else { None }).collect();
    assert!(outputs.iter().any(|l| l.contains("Shop.Api.dll")), "{outputs:?}");
}

#[test]
fn a_broken_copy_reports_parsed_diagnostics() {
    let (_tmp, root) = temp_copy();
    let file = root.join("src/Shop.Core/PriceCalculator.cs");
    let text = std::fs::read_to_string(&file).unwrap();
    let mut lines: Vec<&str> = text.lines().collect();
    let at = lines.iter().position(|l| l.contains("var subtotal = 0m;")).unwrap() + 1;
    lines.insert(at, "        var broken = undefinedName;");
    std::fs::write(&file, lines.join("\n")).unwrap();

    let inbox = Inbox::new(Waker::noop());
    let project = root.join("src/Shop.Core/Shop.Core.csproj");
    let _h = start_build(&dotnet(), BuildRequest::new(&project, BuildAction::Build), inbox.sender());
    let events = collect(&inbox, Duration::from_secs(600), build_finished);
    let r = build_result(&events);
    assert!(!r.success);
    assert!(r.errors >= 1);
    let d = r.errors().find(|d| d.code == "CS0103").unwrap_or_else(|| panic!("{:?}", r.diagnostics));
    assert_eq!(d.file.as_ref().map(|f| f.canonicalize().unwrap()), Some(file.clone()));
    assert_eq!(d.line, Some(at as u32 + 1));
    assert_eq!(d.column, Some(22));
    assert_eq!(d.severity, Severity::Error);
    assert!(d.message.contains("undefinedName"), "{}", d.message);
    assert_eq!(d.project.as_ref().map(|p| p.canonicalize().unwrap()), Some(project.clone()));
    // Diagnostics were streamed while building, not only at the end.
    assert!(events.iter().any(|e| matches!(e, BuildEvent::Diagnostic(x) if x.code == "CS0103")));
}

#[test]
fn cancelling_a_build_kills_it() {
    let (_tmp, root) = temp_copy();
    let inbox = Inbox::new(Waker::noop());
    let started = Instant::now();
    let h = start_build(&dotnet(), BuildRequest::new(root.join("Shop.sln"), BuildAction::Rebuild), inbox.sender());
    let events = collect(&inbox, Duration::from_secs(120), |e| {
        if matches!(e, BuildEvent::Output(_)) {
            h.cancel();
        }
        build_finished(e)
    });
    let r = build_result(&events);
    assert!(r.cancelled && !r.success, "{r:?}");
    assert!(started.elapsed() < Duration::from_secs(60));
}

#[test]
fn runs_the_cli_and_captures_its_output() {
    let _g = lock();
    let d = dotnet();
    let project = shop().join("src/Shop.Cli/Shop.Cli.csproj");
    let mut req = RunRequest::new(&project);
    req.args = vec!["Grace".into(), "--coupon".into(), "SAVE10".into()];
    let inbox = Inbox::new(Waker::noop());
    let h = start_run(&d, req.clone(), inbox.sender());
    let events = collect(&inbox, Duration::from_secs(600), |e| matches!(e, RunEvent::Exited { .. } | RunEvent::Failed(_)));
    let stdout: Vec<&str> = events.iter().filter_map(|e| if let RunEvent::Output(l) = e { (l.stream == Stream::Stdout).then_some(l.text.as_str()) } else { None }).collect();
    assert!(matches!(events.last(), Some(RunEvent::Exited { code: Some(0), cancelled: false, .. })), "{:?}", events.last());
    assert!(stdout.contains(&"Order for Grace (3 items)"), "{stdout:?}");
    assert!(stdout.contains(&"Total: 64.85"), "{stdout:?}");
    assert!(events.iter().any(|e| matches!(e, RunEvent::Output(l) if l.stream == Stream::Stderr && l.text == "done")));
    assert!(events.iter().any(|e| matches!(e, RunEvent::Build(BuildEvent::Finished(r)) if r.success)));
    let started = events.iter().find_map(|e| if let RunEvent::Started { pid, plan } = e { Some((*pid, plan.clone())) } else { None }).unwrap();
    assert!(started.0 > 0);
    assert_eq!(started.1.working_dir.canonicalize().unwrap(), shop().join("src/Shop.Cli"));
    assert!(h.is_finished() || h.pid().is_some());

    // Exit codes are reported; no build this time.
    req.args = vec!["--exit".into(), "3".into()];
    req.build_first = false;
    let inbox = Inbox::new(Waker::noop());
    let _h = start_run(&d, req, inbox.sender());
    let events = collect(&inbox, Duration::from_secs(120), |e| matches!(e, RunEvent::Exited { .. } | RunEvent::Failed(_)));
    assert!(matches!(events.last(), Some(RunEvent::Exited { code: Some(3), .. })), "{:?}", events.last());
    assert!(!events.iter().any(|e| matches!(e, RunEvent::Build(_))));
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn http_get(port: u16, path: &str) -> Option<String> {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").ok()?;
    let mut resp = String::new();
    s.read_to_string(&mut resp).ok()?;
    Some(resp)
}

#[test]
fn starts_the_api_answers_http_and_stops() {
    let _g = lock();
    let port = free_port();
    let url = format!("http://127.0.0.1:{port}");
    let mut req = RunRequest::new(shop().join("src/Shop.Api/Shop.Api.csproj"));
    req.env = vec![("ASPNETCORE_URLS".into(), url.clone())];
    let inbox = Inbox::new(Waker::noop());
    let h = start_run(&dotnet(), req, inbox.sender());

    let events = collect(&inbox, Duration::from_secs(600), |e| matches!(e, RunEvent::Started { .. } | RunEvent::Failed(_) | RunEvent::Exited { .. }));
    let Some(RunEvent::Started { plan, .. }) = events.last() else { panic!("{:?}", events.last()) };
    assert_eq!(plan.profile.as_deref(), Some("http"));
    assert_eq!(plan.urls, vec![url.clone()]);
    assert_eq!(plan.env_var("ASPNETCORE_ENVIRONMENT"), Some("Development"));
    assert_eq!(plan.browser_url.as_deref(), Some(format!("{url}/swagger").as_str()));

    let deadline = Instant::now() + Duration::from_secs(60);
    let health = loop {
        if let Some(r) = http_get(port, "/health") {
            break r;
        }
        assert!(Instant::now() < deadline, "the API never answered: {:?}", inbox.drain());
        std::thread::sleep(Duration::from_millis(250));
    };
    assert!(health.starts_with("HTTP/1.1 200"), "{health}");
    assert!(health.contains("\r\nok\r\n"), "{health}");
    let quote = http_get(port, "/orders/sample?coupon=SAVE10").unwrap();
    assert!(quote.contains("\"total\":64.85"), "{quote}");

    let stop = Instant::now();
    h.stop();
    let events = collect(&inbox, Duration::from_secs(30), |e| matches!(e, RunEvent::Exited { .. } | RunEvent::Failed(_)));
    assert!(matches!(events.last(), Some(RunEvent::Exited { cancelled: true, .. })), "{:?}", events.last());
    assert!(stop.elapsed() < Duration::from_secs(10));
    // The ASP.NET logs went through the console stream.
    assert!(events.iter().any(|e| matches!(e, RunEvent::Output(l) if l.text.contains("Application is shutting down"))), "{events:?}");
    assert!(http_get(port, "/health").is_none(), "still listening after stop");
}

#[test]
fn runs_the_three_test_projects_from_trx() {
    let _g = lock();
    let d = dotnet();
    let inbox = Inbox::new(Waker::noop());
    let _h = start_tests(&d, TestRequest::new(shop().join("Shop.sln")), inbox.sender());
    let events = collect(&inbox, Duration::from_secs(900), |e| matches!(e, TestEvent::Finished(_) | TestEvent::Failed(_)));
    let Some(TestEvent::Finished(r)) = events.last() else { panic!("{:?}", events.last()) };
    assert!(r.success, "{r:#?}");
    assert_eq!(r.projects.len(), 3);
    assert!(r.projects.iter().all(|p| p.exit_code == Some(0) && p.trx.as_ref().is_some_and(|t| t.is_file())));
    assert_eq!((r.passed, r.failed, r.skipped), (13, 0, 0));
    let count = |fqn: &str| r.results.iter().filter(|t| t.fully_qualified_name == fqn).count();
    assert_eq!(count("Shop.Core.XunitTests.PriceCalculatorTests.CouponsGiveDiscounts"), 3);
    assert_eq!(count("Shop.Core.NunitTests.OrderTests.BulkOrdersGetFivePercent"), 2);
    assert_eq!(count("Shop.Core.MSTests.PriceBreakdownTests.SampleKeepsTheCustomer"), 2);
    assert_eq!(count("Shop.Core.MSTests.PriceBreakdownTests.TaxIsRoundedAwayFromZero"), 1);
    assert!(r.results.iter().all(|t| t.outcome == TestOutcome::Passed && !t.class_name.is_empty()));
    let progress = events.iter().filter(|e| matches!(e, TestEvent::Progress(p) if p.outcome == TestOutcome::Passed)).count();
    assert_eq!(progress, 13);

    // Filters, without building again.
    let mut req = TestRequest::new(shop().join("Shop.sln"));
    req.no_build = true;
    req.filter = Some(TestFilter::Tests(vec!["Shop.Core.NunitTests.OrderTests.BulkOrdersGetFivePercent".into(), "Shop.Core.XunitTests.PriceCalculatorTests.EmptyOrderCostsNothing".into()]));
    let inbox = Inbox::new(Waker::noop());
    let _h = start_tests(&d, req.clone(), inbox.sender());
    let events = collect(&inbox, Duration::from_secs(300), |e| matches!(e, TestEvent::Finished(_) | TestEvent::Failed(_)));
    let Some(TestEvent::Finished(r)) = events.last() else { panic!("{:?}", events.last()) };
    assert!(r.build.is_none());
    assert_eq!(r.passed, 3, "{:#?}", r.results);

    req.filter = Some(TestFilter::Class("Shop.Core.MSTests.PriceBreakdownTests".into()));
    let inbox = Inbox::new(Waker::noop());
    let _h = start_tests(&d, req, inbox.sender());
    let events = collect(&inbox, Duration::from_secs(300), |e| matches!(e, TestEvent::Finished(_) | TestEvent::Failed(_)));
    let Some(TestEvent::Finished(r)) = events.last() else { panic!("{:?}", events.last()) };
    assert_eq!(r.passed, 4, "{:#?}", r.results);
    assert!(r.results.iter().all(|t| t.class_name == "Shop.Core.MSTests.PriceBreakdownTests"));
}

#[test]
fn a_failing_test_reports_message_and_stack_location() {
    let (_tmp, root) = temp_copy();
    let file = root.join("tests/Shop.Core.XunitTests/PriceCalculatorTests.cs");
    let text = std::fs::read_to_string(&file).unwrap().replace("Assert.Equal(0m, price.Total);", "Assert.Equal(1m, price.Total);");
    std::fs::write(&file, &text).unwrap();
    let line = text.lines().position(|l| l.contains("Assert.Equal(1m, price.Total);")).unwrap() as u32 + 1;

    let inbox = Inbox::new(Waker::noop());
    let _h = start_tests(&dotnet(), TestRequest::new(root.join("tests/Shop.Core.XunitTests/Shop.Core.XunitTests.csproj")), inbox.sender());
    let events = collect(&inbox, Duration::from_secs(600), |e| matches!(e, TestEvent::Finished(_) | TestEvent::Failed(_)));
    let Some(TestEvent::Finished(r)) = events.last() else { panic!("{:?}", events.last()) };
    assert!(!r.success);
    assert_eq!((r.passed, r.failed), (4, 1));
    let f = r.results.iter().find(|t| t.outcome == TestOutcome::Failed).unwrap();
    assert_eq!(f.fully_qualified_name, "Shop.Core.XunitTests.PriceCalculatorTests.EmptyOrderCostsNothing");
    assert!(f.error_message.as_deref().unwrap_or("").contains("Assert.Equal() Failure"), "{f:?}");
    let frame = f.stack_trace.iter().find(|s| s.file.is_some()).unwrap_or_else(|| panic!("{:?}", f.stack_trace));
    assert_eq!(frame.file.as_ref().unwrap().canonicalize().unwrap(), file);
    assert_eq!(frame.line, Some(line));
    assert!(events.iter().any(|e| matches!(e, TestEvent::Progress(p) if p.outcome == TestOutcome::Failed)));
}

#[test]
fn discovers_tests_with_fully_qualified_names() {
    let _g = lock();
    let tests = Runtime::block_on(discover_tests(&dotnet(), &shop().join("Shop.sln"), "Debug", true, &CancelToken::new())).unwrap();
    let names: Vec<&str> = tests.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"Shop.Core.XunitTests.PriceCalculatorTests.SampleOrderGetsFreeShipping"), "{names:?}");
    assert!(names.contains(&"Shop.Core.NunitTests.OrderTests.BulkOrdersGetFivePercent(9,0)"), "{names:?}");
    assert!(names.contains(&"Shop.Core.MSTests.PriceBreakdownTests.TotalAddsTaxAndShipping"), "{names:?}");
    let projects: std::collections::BTreeSet<_> = tests.iter().map(|t| t.project.clone()).collect();
    assert_eq!(projects.len(), 3);
    let nunit = tests.iter().find(|t| t.name.ends_with("(9,0)")).unwrap();
    assert_eq!((nunit.class_name.as_str(), nunit.method_name.as_str()), ("Shop.Core.NunitTests.OrderTests", "BulkOrdersGetFivePercent"));
}

#[test]
fn adds_and_removes_a_cached_package() {
    // Newtonsoft.Json 13.0.3 is in the global packages folder, so no network is needed.
    let (_tmp, root) = temp_copy();
    let project = root.join("src/Shop.Core/Shop.Core.csproj");
    let d = dotnet();
    Runtime::block_on(async {
        let out = nuget::add_package(&d, &project, "Newtonsoft.Json", Some("13.0.3"), false).await.unwrap();
        assert!(out.success, "{}", out.output);
        let info = rc_dotnet::ProjectInfo::load(&project).unwrap();
        assert!(info.packages.iter().any(|p| p.name == "Newtonsoft.Json" && p.version == "13.0.3"));
        let out = nuget::remove_package(&d, &project, "Newtonsoft.Json").await.unwrap();
        assert!(out.success, "{}", out.output);
        assert!(rc_dotnet::ProjectInfo::load(&project).unwrap().packages.is_empty());
    });
}

#[test]
#[ignore = "needs network access to nuget.org"]
fn searches_nuget_org_and_lists_versions() {
    Runtime::block_on(async {
        let client = nuget::NugetClient::new().unwrap();
        let r = client.search("Newtonsoft.Json", false, 0, 5).await.unwrap();
        assert!(r.total_hits > 0);
        let p = r.packages.iter().find(|p| p.id == "Newtonsoft.Json").unwrap();
        assert!(p.verified && !p.versions.is_empty() && !p.version.contains('-'));
        let pre = client.search("Microsoft.Extensions.AI", true, 0, 20).await.unwrap();
        assert!(!pre.packages.is_empty());
        let versions = client.versions("Newtonsoft.Json").await.unwrap();
        assert!(versions.contains(&"13.0.3".to_string()));
    });
}

/// `dotnet new` through `new_project`: a solution with a console app, then a test project
/// added to it; the solution builds.
#[test]
fn creates_a_solution_and_adds_a_project() {
    use rc_dotnet::new_project::{NewProject, Target};
    let dotnet = rc_dotnet::Dotnet::locate(None).expect("dotnet");
    let dir = tempfile::tempdir().unwrap();
    let fw = rc_dotnet::new_project::frameworks(&dotnet);
    assert!(fw.iter().any(|f| f == "net8.0"), "{fw:?}");
    let p = NewProject { template: "console", project_name: "Demo.App".into(), framework: Some("net8.0".into()), target: Target::NewSolution { location: dir.path().to_path_buf(), solution_name: "Demo".into(), git: true } };
    let sln = rc_core::Runtime::block_on(p.create(&dotnet, &mut |_| {})).unwrap();
    assert!(sln.exists() && dir.path().join("Demo/.git").exists() && dir.path().join("Demo/.gitignore").exists());
    let t = NewProject { template: "xunit", project_name: "Demo.Tests".into(), framework: Some("net8.0".into()), target: Target::AddTo { solution: sln.clone() } };
    rc_core::Runtime::block_on(t.create(&dotnet, &mut |_| {})).unwrap();
    let solution = rc_dotnet::Solution::load(&sln).unwrap();
    let names: Vec<String> = solution.projects().iter().map(|p| p.name.clone()).collect();
    assert!(names.contains(&"Demo.App".to_string()) && names.contains(&"Demo.Tests".to_string()), "{names:?}");
    assert!(rc_dotnet::ProjectInfo::load(&t.project_file()).unwrap().is_test);
    let out = dotnet.std_command().args(["build", "-nologo", "-v", "q"]).arg(&sln).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
}
