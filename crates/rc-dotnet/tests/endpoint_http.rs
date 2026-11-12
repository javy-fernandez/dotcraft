use std::path::Path;

use rc_dotnet::endpoint_http::requests_from_endpoints;
use rc_dotnet::endpoints::scan_project;
use rc_dotnet::http_file::{parse, resolve};
use rc_dotnet::launch::LaunchSettings;

#[test]
fn shop_api_endpoints_become_parseable_http_requests() {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Api");
    let endpoints = scan_project(&project);
    assert_eq!(endpoints.len(), 4);
    let settings = LaunchSettings::load(&project).unwrap().unwrap();
    let base = settings.profiles.iter().find_map(|p| p.application_url.as_deref()).unwrap();
    let generated = requests_from_endpoints(&endpoints, base).unwrap();
    let file = parse(&generated);
    assert_eq!(file.requests.len(), 4);
    let actual: Vec<_> = file.requests.iter().map(|r| (r.method.as_str(), r.url.as_str())).collect();
    assert_eq!(actual, [
        ("GET", "http://localhost:5112/Orders/sample"),
        ("POST", "http://localhost:5112/Orders/quote"),
        ("GET", "http://localhost:5112/WeatherForecast"),
        ("GET", "http://localhost:5112/health"),
    ]);
    for index in 0..4 {
        let request = resolve(&file, index, None, &project.join("generated.http")).unwrap();
        assert_eq!(request.headers, [("Accept".into(), "application/json".into())]);
    }
}

#[test]
fn route_samples_and_method_validation_keep_generated_requests_safe() {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Api");
    let mut endpoint = scan_project(&project).remove(0);
    endpoint.route = "/orders/{id:int}/{key:guid}/{name?}".into();
    endpoint.handler = "Handler\r\nPOST http://evil".into();
    let generated = requests_from_endpoints(&[endpoint.clone()], "https://localhost:5112/").unwrap();
    let file = parse(&generated);
    assert_eq!(file.requests.len(), 1);
    assert_eq!(file.requests[0].url, "https://localhost:5112/orders/1/00000000-0000-0000-0000-000000000001/sample");
    assert!(!generated.contains("\r\nPOST"));

    for route in ["//other-host/path", "/x\r\nPOST http://evil", "/x/{bad", "/x/}", "/x/{bad name}", "/x?token=1"] {
        endpoint.route = route.into();
        assert!(requests_from_endpoints(&[endpoint.clone()], "http://localhost:5112").is_err());
    }
    endpoint.route = "/safe".into();
    endpoint.method = "ANY".into();
    assert!(requests_from_endpoints(&[endpoint.clone()], "http://localhost:5112").is_err());
    endpoint.method = "GET\r\nPOST".into();
    assert!(requests_from_endpoints(&[endpoint.clone()], "http://localhost:5112").is_err());
    endpoint.method = "GET".into();
    for base in ["file:///tmp", "http://user:password@localhost:5112", "http://localhost:5112/api", "http://localhost:5112/?key=1"] {
        assert!(requests_from_endpoints(&[endpoint.clone()], base).is_err());
    }
}
