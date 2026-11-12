//! Live SDK acceptance for Coverlet collection. Run with
//! `DOTCRAFT_TEST_DOTNET=/path/to/dotnet cargo test -p rc-dotnet --test coverage_live -- --ignored`.

use std::path::Path;

use rc_dotnet::coverage::{CoverageEvent, CoverageRequest, has_collector, run_coverage};
use rc_dotnet::{CancelToken, Dotnet};

fn copy_sources(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            if !matches!(
                entry.file_name().to_str(),
                Some("bin" | "obj" | "TestResults")
            ) {
                copy_sources(&path, &target);
            }
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("cs" | "csproj")
        ) {
            std::fs::copy(&path, target).unwrap();
        }
    }
}

#[tokio::test]
#[ignore = "requires .NET 8 SDK and the fixture's Coverlet/xUnit NuGet packages"]
async fn collects_cobertura_for_real_shop_xunit_tests() {
    let host = std::env::var_os("DOTCRAFT_TEST_DOTNET")
        .expect("set DOTCRAFT_TEST_DOTNET to a .NET 8 dotnet host");
    let dotnet = Dotnet::new(host);
    assert!(dotnet.version(None).unwrap().starts_with("8."));

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop");
    let dir = tempfile::tempdir().unwrap();
    let shop = dir.path().join("Shop");
    copy_sources(&fixture.join("src/Shop.Core"), &shop.join("src/Shop.Core"));
    copy_sources(
        &fixture.join("tests/Shop.Core.XunitTests"),
        &shop.join("tests/Shop.Core.XunitTests"),
    );
    let project = shop.join("tests/Shop.Core.XunitTests/Shop.Core.XunitTests.csproj");
    let source = shop.join("src/Shop.Core/PriceCalculator.cs");
    assert!(has_collector(&project));

    let mut request = CoverageRequest::new(&project);
    request.results_dir = Some(dir.path().join("coverage-results"));
    request.max_parallel = 1;
    let mut events = Vec::new();
    let result = run_coverage(&dotnet, &request, &CancelToken::new(), &mut |event| {
        events.push(event)
    })
    .await
    .unwrap();

    assert!(result.build.as_ref().is_some_and(|build| build.success));
    assert!(result.tests_passed && !result.cancelled, "{result:?}");
    assert!(result.skipped.is_empty());
    assert_eq!(result.projects.len(), 1);
    assert_eq!(result.projects[0].project, project);
    assert_eq!(result.projects[0].exit_code, Some(0));
    assert_eq!(result.projects[0].reports.len(), 1);
    assert!(result.projects[0].reports[0].ends_with("coverage.cobertura.xml"));
    assert!(result.projects[0].reports[0].is_file());
    assert!(result.report.total.total > 0 && result.report.total.covered > 0);
    assert!(
        result
            .report
            .file(&source)
            .is_some_and(|file| file.ratio.total > 0 && file.ratio.covered > 0)
    );
    assert!(events.iter().any(
        |event| matches!(event, CoverageEvent::ProjectStarted { project: p } if p == &project)
    ));
    assert!(events.iter().any(|event| matches!(event, CoverageEvent::ProjectFinished(run) if run.project == project && run.exit_code == Some(0))));
}
