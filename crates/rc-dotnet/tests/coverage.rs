//! Coverage with coverlet against a temporary copy of `tests/fixtures/Shop`.

use std::path::Path;
use std::time::{Duration, Instant};

use rc_core::{Inbox, Waker};
use rc_dotnet::Dotnet;
use rc_dotnet::coverage::{CoverageEvent, LineState, start_coverage};

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

#[test]
#[ignore = "Requires the .NET SDK"]
fn collects_coverage_of_the_shop_tests() {
    let dotnet = Dotnet::locate(None).expect("the dotnet SDK must be installed");
    let dir = tempfile::Builder::new().prefix("dotcraft-cov").tempdir().unwrap();
    let root = dunce::canonicalize(dir.path()).unwrap().join("Shop");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop"), &root);

    let inbox = Inbox::new(Waker::noop());
    let _handle = start_coverage(&dotnet, &root.join("Shop.sln"), inbox.sender());
    let deadline = Instant::now() + Duration::from_secs(600);
    let mut warnings = Vec::new();
    let mut output = Vec::new();
    let result = loop {
        assert!(Instant::now() < deadline, "no final event");
        match inbox.recv_timeout(Duration::from_millis(200)) {
            Some(CoverageEvent::Finished(r)) => break r,
            Some(CoverageEvent::Failed(e)) => panic!("{e}"),
            Some(CoverageEvent::Warning(w)) => warnings.push(w),
            Some(CoverageEvent::Output { line, .. }) => output.push(line.text),
            _ => {}
        }
    };
    assert!(result.build.as_ref().is_some_and(|b| b.success), "build failed");
    // The MSTest project of the fixture does not reference coverlet.collector.
    assert_eq!(result.skipped.len(), 1, "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("coverlet.collector")));
    assert_eq!(result.projects.len(), 2);
    assert!(result.projects.iter().all(|p| p.reports.len() == 1), "{:?}\n{}", result.projects, output.join("\n"));

    let calc = root.join("src/Shop.Core/PriceCalculator.cs");
    let file = result.report.file(&calc).unwrap_or_else(|| panic!("{calc:?} not in {:?}", result.report.files.keys().collect::<Vec<_>>()));
    assert!(file.ratio.covered > 0 && file.ratio.total >= file.ratio.covered);
    // `subtotal += line.Subtotal;` runs for every order line.
    let text = std::fs::read_to_string(&calc).unwrap();
    let loop_line = text.lines().position(|l| l.contains("subtotal += line.Subtotal")).unwrap();
    assert_eq!(file.line_state(loop_line), Some(LineState::Covered));
    assert!(result.report.total.covered > 0);
    assert!(result.report.assemblies.iter().any(|a| a.name == "Shop.Core" && a.ratio.covered > 0));
    assert!(result.report.classes.iter().any(|c| c.name == "Shop.Core.PriceCalculator" && c.ratio.covered > 0));
}
