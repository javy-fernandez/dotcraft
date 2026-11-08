//! Bounded, offline OpenAPI 3 JSON to editable HTTP request templates.
//! No references are fetched and generated requests are never executed.
use serde_json::Value;

const METHODS: &[&str] = &["get", "post", "put", "patch", "delete", "head", "options", "trace"];

pub fn requests(source: &str) -> anyhow::Result<String> {
    anyhow::ensure!(source.len() <= 2 * 1024 * 1024, "OpenAPI input exceeds 2 MiB");
    let doc: Value = serde_json::from_str(source)?;
    let version = doc["openapi"].as_str().unwrap_or_default();
    anyhow::ensure!(version.starts_with("3.0.") || version.starts_with("3.1."), "Only OpenAPI 3.0 and 3.1 JSON documents are supported");
    let paths = doc["paths"].as_object().ok_or_else(|| anyhow::anyhow!("OpenAPI paths must be an object"))?;
    let mut output = String::from("# Imported OpenAPI templates: review example values and authentication before sending.\n# Authentication and response schemas are not imported.\n# Parameters without examples use placeholders; JSON bodies without inline examples use {}.\n\n");
    let mut count = 0;
    for (path, item) in paths {
        if path.starts_with("x-") { continue; }
        anyhow::ensure!(path.len() <= 8192 && path.starts_with('/') && !path.chars().any(|c| c.is_whitespace() || c.is_control()) && !path.contains(['?', '#']), "Invalid or oversized OpenAPI path");
        reject_ref(item)?;
        for method in METHODS {
            let Some(operation) = item.get(*method) else { continue };
            anyhow::ensure!(operation.is_object(), "Operation must be an object");
            count += 1;
            anyhow::ensure!(count <= 512, "OpenAPI import exceeds 512 operations");
            let servers = operation.get("servers").or_else(|| item.get("servers")).or_else(|| doc.get("servers"));
            let base = server(servers)?;
            let mut route = path.clone(); let mut query = Vec::new(); let mut headers = Vec::new();
            let mut parameters: std::collections::BTreeMap<(String, String), &Value> = Default::default();
            for owner in [item, operation] {
                if let Some(params) = owner.get("parameters") {
                    let params = params.as_array().ok_or_else(|| anyhow::anyhow!("Parameters must be an array"))?;
                    anyhow::ensure!(params.len() <= 256, "Too many OpenAPI parameters");
                    for parameter in params {
                        reject_ref(parameter)?;
                        let name = parameter["name"].as_str().ok_or_else(|| anyhow::anyhow!("Parameter name is missing"))?;
                        let location = parameter["in"].as_str().ok_or_else(|| anyhow::anyhow!("Parameter location is missing"))?;
                        parameters.insert((location.into(), name.into()), parameter);
                    }
                }
            }
            for ((location, name), parameter) in parameters {
                anyhow::ensure!(name.len() <= 256 && !name.contains(['\r', '\n']), "Invalid parameter name");
                anyhow::ensure!(parameter.get("content").is_none() && parameter.get("examples").is_none(), "Parameter content and named examples are not supported; provide an inline scalar example");
                anyhow::ensure!(!matches!(parameter["schema"]["type"].as_str(), Some("array" | "object")), "Array and object parameters are not supported");
                let style = if location == "query" { "form" } else { "simple" };
                anyhow::ensure!(parameter.get("style").is_none_or(|s| s.as_str() == Some(style)), "Only simple path/header and form query serialization is supported");
                let sample = parameter.get("example").or_else(|| parameter["schema"].get("default")).or_else(|| parameter["schema"].get("example"));
                let value = match sample { Some(Value::String(s)) => s.clone(), Some(v) if v.is_number() || v.is_boolean() => v.to_string(), Some(_) => anyhow::bail!("Only scalar parameter examples are supported"), None => if parameter["schema"]["type"] == "integer" { "1".into() } else { "value".into() } };
                anyhow::ensure!(value.len() <= 2048, "Parameter examples must not exceed 2048 bytes");
                match location.as_str() {
                    "path" => route = route.replace(&format!("{{{name}}}"), &encode(&value)),
                    "query" => query.push(format!("{}={}", encode(&name), encode(&value))),
                    "header" => {
                        anyhow::ensure!(!name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') && !value.contains(['\r', '\n']), "Invalid header parameter");
                        headers.push(format!("{name}: {value}\n"));
                    }
                    _ => anyhow::bail!("Cookie and other parameter locations are not supported"),
                }
                anyhow::ensure!(route.len() <= 64 * 1024, "Generated route exceeds 64 KiB");
            }
            anyhow::ensure!(!route.contains(['{', '}']), "Every path parameter needs an inline parameter declaration");
            let label = operation["operationId"].as_str().or_else(|| operation["summary"].as_str()).unwrap_or(path);
            let label: String = label.chars().take(200).map(|c| if c.is_control() { ' ' } else { c }).collect();
            output.push_str(&format!("### {label}\n{} {}{}", method.to_uppercase(), base.trim_end_matches('/'), route));
            if !query.is_empty() { output.push('?'); output.push_str(&query.join("&")); }
            output.push_str("\nAccept: application/json\n");
            for header in headers { output.push_str(&header); }
            if let Some(body) = operation.get("requestBody") {
                reject_ref(body)?;
                let json = body.pointer("/content/application~1json").ok_or_else(|| anyhow::anyhow!("Only application/json request bodies are supported"))?;
                let value = json.get("example").cloned().unwrap_or_else(|| serde_json::json!({}));
                output.push_str("Content-Type: application/json\n\n"); output.push_str(&serde_json::to_string_pretty(&value)?); output.push('\n');
            }
            output.push('\n');
            anyhow::ensure!(output.len() <= 4 * 1024 * 1024, "Generated HTTP file exceeds 4 MiB");
        }
    }
    anyhow::ensure!(count != 0, "OpenAPI contains no supported operations");
    Ok(output)
}
fn reject_ref(value: &Value) -> anyhow::Result<()> {
    anyhow::ensure!(value.get("$ref").is_none(), "Referenced path items, parameters and request bodies are not supported; inline them before import"); Ok(())
}
fn server(servers: Option<&Value>) -> anyhow::Result<String> {
    let server = servers.and_then(Value::as_array).and_then(|s| s.first()).ok_or_else(|| anyhow::anyhow!("An explicit HTTP(S) server URL is required"))?;
    let mut url = server["url"].as_str().ok_or_else(|| anyhow::anyhow!("Server URL is missing"))?.to_owned();
    anyhow::ensure!(url.len() <= 8192, "Server URL exceeds 8192 bytes");
    if let Some(vars) = server["variables"].as_object() {
        for (name, variable) in vars {
            let value = variable["default"].as_str().ok_or_else(|| anyhow::anyhow!("Server variables require string defaults"))?;
            anyhow::ensure!(value.len() <= 2048, "Server variable defaults exceed 2048 bytes");
            url = url.replace(&format!("{{{name}}}"), value);
            anyhow::ensure!(url.len() <= 8192, "Server URL exceeds 8192 bytes");
        }
    }
    anyhow::ensure!(!url.contains(['{', '}', '\r', '\n', ' ']), "Unresolved or invalid server URL");
    let parsed = reqwest::Url::parse(&url)?;
    anyhow::ensure!(matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some() && parsed.username().is_empty() && parsed.password().is_none() && parsed.query().is_none() && parsed.fragment().is_none(), "Server must be an absolute HTTP(S) URL without credentials, query or fragment");
    Ok(url)
}
fn encode(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn openapi_templates_preserve_overrides_parameters_and_json_examples() {
        let doc = serde_json::json!({"openapi":"3.1.0","servers":[{"url":"https://{host}/api","variables":{"host":{"default":"example.com"}}}],"paths":{"/items/{id}":{"parameters":[{"name":"id","in":"path","schema":{"type":"integer"}}],"get":{"parameters":[{"name":"q","in":"query","example":"a b"},{"name":"id","in":"path","example":7}]},"post":{"servers":[{"url":"http://localhost:123"}],"requestBody":{"content":{"application/json":{"example":{"name":"value"}}}}}}}});
        let text = requests(&doc.to_string()).unwrap();
        let parsed = crate::http_file::parse(&text);
        assert_eq!(parsed.requests.len(), 2);
        assert!(text.contains("GET https://example.com/api/items/7?q=a%20b"));
        assert!(text.contains("POST http://localhost:123/items/1"));
        assert!(text.contains("\"name\": \"value\""));
    }
    #[test]
    fn openapi_rejects_unsupported_and_unsafe_templates() {
        for doc in [serde_json::json!({"openapi":"2.0"}), serde_json::json!({"openapi":"3.1.0","paths":{"/x":{"$ref":"https://host/spec"}}}), serde_json::json!({"openapi":"3.1.0","servers":[{"url":"file:///tmp"}],"paths":{"/x":{"get":{}}}}), serde_json::json!({"openapi":"3.1.0","servers":[{"url":"http://localhost"}],"paths":{"/x":{"get":{"parameters":[{"name":"X-Test","in":"header","example":"ok\r\nInjected: yes"}]}}}})] {
            assert!(requests(&doc.to_string()).is_err());
        }
        assert!(requests(&" ".repeat(2*1024*1024+1)).is_err());
    }
    #[test]
    fn openapi_reports_unsupported_serialization_bodies_and_operation_limit() {
        for operation in [serde_json::json!({"parameters":[{"name":"ids","in":"query","schema":{"type":"array"}}]}), serde_json::json!({"requestBody":{"content":{"multipart/form-data":{}}}}), serde_json::json!({"parameters":[{"$ref":"#/components/parameters/id"}]})] {
            let doc = serde_json::json!({"openapi":"3.0.3","servers":[{"url":"https://example.com"}],"paths":{"/items":{"post":operation}}});
            assert!(requests(&doc.to_string()).is_err());
        }
        let paths: serde_json::Map<String, Value> = (0..513).map(|n| (format!("/item/{n}"), serde_json::json!({"get":{}}))).collect();
        let doc = serde_json::json!({"openapi":"3.0.3","servers":[{"url":"https://example.com"}],"paths":paths});
        assert!(requests(&doc.to_string()).unwrap_err().to_string().contains("512 operations"));
    }
}
