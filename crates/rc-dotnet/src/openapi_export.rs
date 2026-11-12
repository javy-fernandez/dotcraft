//! Bounded OpenAPI 3 skeletons from statically discovered ASP.NET endpoints.

use serde_json::{Value, json};

use crate::endpoints::Endpoint;

const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE"];

/// Exports route and method metadata, without inferring request or response schemas.
pub fn export(endpoints: &[Endpoint], server_url: &str, title: &str) -> anyhow::Result<String> {
    anyhow::ensure!(!endpoints.is_empty() && endpoints.len() <= 512, "Provide 1 to 512 endpoints");
    anyhow::ensure!(!title.trim().is_empty() && title.len() <= 120 && !title.chars().any(char::is_control), "OpenAPI title is invalid or oversized");
    anyhow::ensure!(server_url.len() <= 8192 && !server_url.chars().any(char::is_control), "Server URL is invalid or oversized");
    let server = reqwest::Url::parse(server_url)?;
    anyhow::ensure!(matches!(server.scheme(), "http" | "https") && server.host_str().is_some(), "An absolute HTTP(S) server URL is required");
    anyhow::ensure!(server.username().is_empty() && server.password().is_none() && server.query().is_none() && server.fragment().is_none() && server.path() == "/", "Server URL must be an origin without credentials, path, query or fragment");

    let mut paths = serde_json::Map::new();
    let mut count = 0;
    for endpoint in endpoints {
        let method = endpoint.method.as_str();
        if !METHODS.contains(&method) { continue; }
        let (path, parameters) = path_and_parameters(&endpoint.route)?;
        let entry = paths.entry(path).or_insert_with(|| json!({}));
        let item = entry.as_object_mut().unwrap();
        let operation = json!({
            "summary": endpoint.handler,
            "parameters": parameters,
            "responses": {"default": {"description": "Response"}}
        });
        anyhow::ensure!(item.insert(method.to_ascii_lowercase(), operation).is_none(), "Duplicate HTTP method and path in OpenAPI export");
        count += 1;
    }
    anyhow::ensure!(count > 0, "No endpoint has an explicit supported HTTP method");
    let doc = json!({"openapi": "3.0.3", "info": {"title": title, "version": "1.0.0"}, "servers": [{"url": server.as_str().trim_end_matches('/')}], "paths": paths});
    let output = format!("{}\n", serde_json::to_string_pretty(&doc)?);
    anyhow::ensure!(output.len() <= 2 * 1024 * 1024, "OpenAPI export exceeds 2 MiB");
    Ok(output)
}

fn path_and_parameters(route: &str) -> anyhow::Result<(String, Vec<Value>)> {
    anyhow::ensure!(route.len() <= 8192 && route.starts_with('/') && !route.starts_with("//") && !route.contains(['?', '#', '[', ']']) && !route.chars().any(|c| c.is_control() || c.is_whitespace()), "Endpoint route is invalid or oversized");
    let mut path = String::new();
    let mut parameters = Vec::new();
    let mut chars = route.chars();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                let mut token = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some('{') | None => anyhow::bail!("Endpoint route has an invalid parameter"),
                        Some(c) => token.push(c),
                    }
                }
                anyhow::ensure!(!token.contains(['?', '=', '*']), "Optional, default and catch-all route parameters are unsupported");
                let (name, constraint) = token.split_once(':').unwrap_or((&token, "string"));
                anyhow::ensure!(!name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'), "Endpoint route has an invalid parameter name");
                anyhow::ensure!(!parameters.iter().any(|p: &Value| p["name"] == name), "Duplicate route parameter");
                let (kind, format, example) = match constraint.to_ascii_lowercase().as_str() {
                    "int" | "long" => ("integer", None, json!(1)),
                    "guid" => ("string", Some("uuid"), json!("00000000-0000-0000-0000-000000000001")),
                    "bool" => ("boolean", None, json!(true)),
                    "alpha" | "string" => ("string", None, json!("sample")),
                    _ => anyhow::bail!("Unsupported route parameter constraint: {constraint}"),
                };
                let mut schema = json!({"type": kind});
                if let Some(format) = format { schema["format"] = json!(format); }
                parameters.push(json!({"name": name, "in": "path", "required": true, "schema": schema, "example": example}));
                path.push_str(&format!("{{{name}}}"));
            }
            '}' => anyhow::bail!("Endpoint route has an unmatched closing brace"),
            _ => path.push(c),
        }
    }
    Ok((path, parameters))
}
