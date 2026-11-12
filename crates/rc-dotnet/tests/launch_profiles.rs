use rc_dotnet::{LaunchProfile, LaunchSettings};
use std::path::Path;

#[test]
fn shop_api_profiles_keep_project_order_urls_and_environment() {
    let project = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/Shop/src/Shop.Api/Shop.Api.csproj");
    let settings = LaunchSettings::load(&project).unwrap().unwrap();
    assert_eq!(
        settings
            .profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>(),
        vec!["http", "IIS Express"]
    );
    let profile = settings.default_profile().unwrap();
    assert_eq!(profile.name, "http");
    assert_eq!(profile.urls(), vec!["http://localhost:5112"]);
    assert_eq!(
        profile.browser_url().as_deref(),
        Some("http://localhost:5112/swagger")
    );
    assert_eq!(
        profile.environment_variables,
        vec![("ASPNETCORE_ENVIRONMENT".into(), "Development".into())]
    );
    assert!(profile.dotnet_run_messages);
    assert!(!settings.profile("IIS Express").unwrap().is_project());
    assert_eq!(settings.profile("IIS Express").unwrap().browser_url(), None);
    assert_eq!(
        LaunchSettings::load(project.parent().unwrap()).unwrap(),
        Some(settings)
    );
}

#[test]
fn browser_url_requires_a_scheme_at_the_start() {
    let base = LaunchProfile {
        application_url: Some("https://localhost:7001".into()),
        launch_browser: true,
        ..Default::default()
    };
    for (launch_url, expected) in [
        (
            "swagger?next=https://docs.example/path",
            "https://localhost:7001/swagger?next=https://docs.example/path",
        ),
        (
            "path/http://docs.example",
            "https://localhost:7001/path/http://docs.example",
        ),
        ("1invalid://host", "https://localhost:7001/1invalid://host"),
        (
            "https://docs.example/swagger",
            "https://docs.example/swagger",
        ),
        ("HTTPS://docs.example/swagger", "HTTPS://docs.example/swagger"),
        ("custom+app://launch", "custom+app://launch"),
    ] {
        let profile = LaunchProfile {
            launch_url: Some(launch_url.into()),
            ..base.clone()
        };
        assert_eq!(
            profile.browser_url().as_deref(),
            Some(expected),
            "{launch_url}"
        );
    }
    let inactive = LaunchProfile {
        launch_browser: false,
        launch_url: Some("https://docs.example".into()),
        ..base
    };
    assert_eq!(inactive.browser_url(), None);
}
