//! .NET Aspire app hosts: recognising them, the environment the IDE starts them with, the
//! dashboard login URL they print, the IDE session endpoint their orchestrator (DCP) calls to
//! run the services under the debugger ([`ide_session`]) and the resource service the
//! dashboard reads ([`resources`]). Nothing here needs the `aspire` CLI.

pub mod ide_session;
pub mod proto;
pub mod resources;

pub use ide_session::{IdeSessionServer, Notifier, RunSession, SessionRequest};
pub use resources::{Health, LogLine, Resource, ResourceEndpoint, ResourceEvent, ResourceService, WatchHandle};

use crate::launch::LaunchSettings;
use crate::run::RunRequest;

/// Resource service URL variables (Aspire 13+, then 8/9).
const RESOURCE_SERVICE_URL: [&str; 2] = ["ASPIRE_RESOURCE_SERVICE_ENDPOINT_URL", "DOTNET_RESOURCE_SERVICE_ENDPOINT_URL"];
/// The resource service API key the app host uses (and gives the dashboard).
const RESOURCE_SERVICE_KEY: [&str; 2] = ["ASPIRE_DASHBOARD_RESOURCESERVICE_APIKEY", "DOTNET_DASHBOARD_RESOURCESERVICE_APIKEY"];
const ALLOW_UNSECURED: [&str; 2] = ["ASPIRE_ALLOW_UNSECURED_TRANSPORT", "DOTNET_ALLOW_UNSECURED_TRANSPORT"];

/// 128 random bits as hex (bearer token, API key).
pub fn random_token() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // Practically unreachable; still unpredictable enough for a loopback-only secret.
        use std::hash::{BuildHasher, Hasher};
        for chunk in bytes.chunks_mut(8) {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
            chunk.copy_from_slice(&h.finish().to_le_bytes()[..chunk.len()]);
        }
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The dashboard login URL in a line the app host printed ("Login to the dashboard at …").
pub fn login_url(line: &str) -> Option<String> {
    let line = crate::process::strip_ansi(line);
    let (_, rest) = line.split_once("Login to the dashboard at ")?;
    let url = rest.split_whitespace().next()?.trim_end_matches(['.', ',']);
    (url.starts_with("http://") || url.starts_with("https://")).then(|| url.to_string())
}

/// The variables an app host is started with so the IDE can read its resources: a known
/// resource service endpoint (the launch profile's, or a free port) and a fresh API key.
/// Returns them with the endpoint to connect to.
pub fn host_environment(request: &RunRequest) -> (Vec<(String, String)>, ResourceEndpoint) {
    let profile_env: Vec<(String, String)> = if request.use_launch_profile {
        LaunchSettings::load(&request.project)
            .ok()
            .flatten()
            .and_then(|s| match &request.launch_profile {
                Some(name) => s.profile(name).cloned(),
                None => s.default_profile().cloned(),
            })
            .map(|p| p.environment_variables)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    // The request's variables win over the profile's, as when the app is started.
    let lookup = |names: &[&str]| {
        names.iter().find_map(|n| request.env.iter().rev().chain(profile_env.iter().rev()).find(|(k, _)| k == n).map(|(_, v)| v.trim().to_string())).filter(|v| !v.is_empty())
    };
    let mut env = Vec::new();
    let url = match lookup(&RESOURCE_SERVICE_URL) {
        Some(url) => url.split(';').next().unwrap_or_default().to_string(),
        None => {
            let unsecured = lookup(&ALLOW_UNSECURED).is_some_and(|v| v.eq_ignore_ascii_case("true"));
            let port = std::net::TcpListener::bind(("127.0.0.1", 0)).and_then(|l| l.local_addr()).map(|a| a.port()).unwrap_or(22057);
            let url = format!("{}://localhost:{port}", if unsecured { "http" } else { "https" });
            env.push((RESOURCE_SERVICE_URL[0].to_string(), url.clone()));
            url
        }
    };
    let api_key = random_token();
    for name in RESOURCE_SERVICE_KEY {
        env.push((name.to_string(), api_key.clone()));
    }
    (env, ResourceEndpoint { url, api_key })
}

/// The request DotCraft runs for a run session DCP asked for. The app host already built its
/// projects (they are its references), so they are not built again: parallel builds of shared
/// projects would collide.
pub fn run_request(session: &RunSession) -> RunRequest {
    let mut request = RunRequest::new(&session.project);
    if let Some(configuration) = &session.configuration {
        request.configuration = configuration.clone();
    }
    request.launch_profile = session.launch_profile.clone();
    request.use_launch_profile = !session.disable_launch_profile;
    // The profile is optional: a missing one means no profile, not an error.
    if !request.use_launch_profile || request.launch_profile.as_ref().is_some_and(|name| !LaunchSettings::load(&session.project).ok().flatten().is_some_and(|s| s.profile(name).is_some())) {
        request.launch_profile = None;
        request.use_launch_profile = false;
    }
    request.env = session.env.clone();
    if let Some(args) = &session.args {
        request.args = args.clone();
    }
    request.build_first = false;
    request
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_login_url_in_app_host_output() {
        assert_eq!(
            login_url("      Login to the dashboard at https://localhost:17175/login?t=2e2d75491b1f144e55a20b586e11e3c9"),
            Some("https://localhost:17175/login?t=2e2d75491b1f144e55a20b586e11e3c9".into())
        );
        assert_eq!(login_url("\x1b[32minfo\x1b[0m: Login to the dashboard at http://localhost:15137/login?t=abc\r"), Some("http://localhost:15137/login?t=abc".into()));
        assert_eq!(login_url("Now listening on: https://localhost:17175"), None);
    }

    #[test]
    fn tokens_are_random_hex() {
        let (a, b) = (random_token(), random_token());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn host_environment_uses_the_profile_endpoint_or_picks_one() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("AppHost.csproj");
        std::fs::write(&project, "<Project Sdk=\"Microsoft.NET.Sdk\"/>").unwrap();
        std::fs::create_dir_all(dir.path().join("Properties")).unwrap();
        std::fs::write(
            dir.path().join("Properties/launchSettings.json"),
            r#"{ "profiles": {
                "https": { "commandName": "Project", "environmentVariables": { "ASPIRE_RESOURCE_SERVICE_ENDPOINT_URL": "https://localhost:22232" } },
                "http": { "commandName": "Project", "environmentVariables": { "ASPIRE_ALLOW_UNSECURED_TRANSPORT": "true" } } } }"#,
        )
        .unwrap();
        let request = RunRequest::new(&project);
        let (env, endpoint) = host_environment(&request);
        assert_eq!(endpoint.url, "https://localhost:22232");
        assert!(env.iter().all(|(k, _)| !k.contains("ENDPOINT_URL")), "the profile's endpoint is kept");
        assert!(env.contains(&("ASPIRE_DASHBOARD_RESOURCESERVICE_APIKEY".into(), endpoint.api_key.clone())));
        assert!(env.contains(&("DOTNET_DASHBOARD_RESOURCESERVICE_APIKEY".into(), endpoint.api_key.clone())));

        let mut http = RunRequest::new(&project);
        http.launch_profile = Some("http".into());
        let (env, endpoint) = host_environment(&http);
        assert!(endpoint.url.starts_with("http://localhost:"), "{endpoint:?}");
        assert!(env.contains(&("ASPIRE_RESOURCE_SERVICE_ENDPOINT_URL".into(), endpoint.url.clone())));
    }

    #[test]
    fn run_sessions_become_run_requests_without_a_build() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("Api.csproj");
        std::fs::write(&project, "<Project Sdk=\"Microsoft.NET.Sdk.Web\"/>").unwrap();
        let session = RunSession {
            project: project.clone(),
            debug: true,
            launch_profile: Some("https".into()),
            disable_launch_profile: false,
            configuration: Some("Release".into()),
            env: vec![("ASPNETCORE_URLS".into(), "http://localhost:5000".into())],
            args: Some(vec![]),
        };
        let request = run_request(&session);
        assert!(!request.build_first);
        assert_eq!(request.configuration, "Release");
        // No launchSettings.json: the named profile does not exist, so none is used.
        assert!(!request.use_launch_profile);
        assert_eq!(request.env, session.env);
    }
}
