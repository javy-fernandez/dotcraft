use std::path::Path;

use rc_dotnet::endpoints::scan_project;
use rc_dotnet::http_file::parse;
use rc_dotnet::launch::LaunchSettings;
use rc_dotnet::openapi;
use rc_dotnet::openapi_export::export;

#[test]
fn shop_api_endpoints_export_and_import_as_openapi_three() {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Api");
    let endpoints = scan_project(&project);
    assert_eq!(endpoints.len(), 4);
    let settings = LaunchSettings::load(&project).unwrap().unwrap();
    let server = settings.profiles.iter().find_map(|p| p.application_url.as_deref()).unwrap();
    let exported = export(&endpoints, server, "Shop API").unwrap();
    let doc: serde_json::Value = serde_json::from_str(&exported).unwrap();
    assert_eq!(doc["openapi"], "3.0.3");
    assert_eq!(doc["info"]["title"], "Shop API");
    assert_eq!(doc["paths"].as_object().unwrap().len(), 4);
    assert!(doc["paths"]["/Orders/quote"]["post"]["responses"]["default"].is_object());

    let requests = parse(&openapi::requests(&exported).unwrap());
    assert_eq!(requests.requests.len(), 4);
    let urls: Vec<_> = requests.requests.iter().map(|r| (r.method.as_str(), r.url.as_str())).collect();
    for expected in [
        ("GET", "http://localhost:5112/Orders/sample"),
        ("POST", "http://localhost:5112/Orders/quote"),
        ("GET", "http://localhost:5112/WeatherForecast"),
        ("GET", "http://localhost:5112/health"),
    ] {
        assert!(urls.contains(&expected));
    }
}

#[test]
fn route_parameters_and_unsafe_inputs_are_validated() {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Api");
    let mut endpoint = scan_project(&project).remove(0);
    endpoint.route = "/orders/{id:int}/{key:guid}".into();
    let exported = export(&[endpoint.clone()], "https://localhost:5112", "Orders").unwrap();
    let doc: serde_json::Value = serde_json::from_str(&exported).unwrap();
    let parameters = doc["paths"]["/orders/{id}/{key}"]["get"]["parameters"].as_array().unwrap();
    assert_eq!(parameters.len(), 2);
    assert_eq!(parameters[0]["schema"]["type"], "integer");
    assert_eq!(parameters[1]["schema"]["format"], "uuid");
    assert!(openapi::requests(&exported).unwrap().contains("/orders/1/00000000-0000-0000-0000-000000000001"));

    assert!(export(&[endpoint.clone(), endpoint.clone()], "http://localhost:5112", "Orders").is_err());
    for route in ["//evil/path", "/x\r\nPOST", "/x/{id?}", "/x/{*path}", "/x/{id:regex(.*)}", "/x?token=1"] {
        endpoint.route = route.into();
        assert!(export(&[endpoint.clone()], "http://localhost:5112", "Orders").is_err());
    }
    endpoint.route = "/safe".into();
    endpoint.method = "GET\r\nPOST".into();
    assert!(export(&[endpoint.clone()], "http://localhost:5112", "Orders").is_err());
    endpoint.method = "GET".into();
    for server in ["file:///tmp", "http://user:pass@localhost", "http://localhost/api", "http://localhost?token=1"] {
        assert!(export(&[endpoint.clone()], server, "Orders").is_err());
    }
}
