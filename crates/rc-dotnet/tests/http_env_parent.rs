use std::path::{Path, PathBuf};

use rc_dotnet::http_file::{environment_variables, environments, parse, resolve};

fn shop_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop")
}

#[test]
fn shop_api_http_uses_parent_public_and_child_private_environment() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path();
    std::fs::write(outside.join("http-client.env.json"), r#"{"outside":{"OnlyOutside":"hidden"}}"#).unwrap();
    let root = outside.join("Shop");
    let api = root.join("src/Shop.Api");
    std::fs::create_dir_all(&api).unwrap();
    std::fs::copy(shop_fixture().join("Shop.sln"), root.join("Shop.sln")).unwrap();
    std::fs::write(root.join("http-client.env.json"), r#"{"$shared":{"region":"us"},"dev":{"Shop.Api_HostAddress":"http://localhost:5112","token":"public"},"prod":{"Shop.Api_HostAddress":"https://shop.example"}}"#).unwrap();
    std::fs::write(root.join("src/http-client.env.json"), r#"{"dev":{"Shop.Api_HostAddress":"http://localhost:5222"}}"#).unwrap();
    std::fs::write(api.join("http-client.private.env.json"), r#"{"dev":{"token":"private"},"local":{"Shop.Api_HostAddress":"http://localhost:5333"}}"#).unwrap();
    std::fs::write(api.join("http-client.env.json.user"), r#"{"dev":{"token":"user"}}"#).unwrap();

    let source = std::fs::read_to_string(shop_fixture().join("src/Shop.Api/Shop.Api.http")).unwrap();
    let request = source.replacen("@Shop.Api_HostAddress = http://localhost:5112", "# Host address is supplied by the selected environment", 1);
    let http = api.join("Shop.Api.http");
    std::fs::write(&http, &request).unwrap();

    let names = environments(&http);
    assert!(names.contains(&"dev".to_string()));
    assert!(names.contains(&"prod".to_string()));
    assert!(names.contains(&"local".to_string()));
    assert!(!names.contains(&"outside".to_string()));

    let vars = environment_variables(&http, Some("dev"));
    assert_eq!(vars["Shop.Api_HostAddress"], "http://localhost:5222");
    assert_eq!(vars["token"], "user");
    assert_eq!(vars["region"], "us");
    assert!(!vars.contains_key("OnlyOutside"));
    let resolved = resolve(&parse(&request), 0, Some("dev"), &http).unwrap();
    assert_eq!(resolved.url, "http://localhost:5222/weatherforecast/");
    assert_eq!(resolved.headers, vec![("Accept".into(), "application/json".into())]);
    assert_eq!(environment_variables(&http, Some("prod"))["Shop.Api_HostAddress"], "https://shop.example");
}
