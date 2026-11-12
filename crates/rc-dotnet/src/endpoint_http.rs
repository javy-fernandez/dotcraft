//! Offline `.http` request templates from discovered ASP.NET endpoints.

use crate::endpoints::Endpoint;

const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE"];

/// Generates editable requests for endpoints with explicit HTTP methods.
pub fn requests_from_endpoints(endpoints: &[Endpoint], base_url: &str) -> anyhow::Result<String> {
    anyhow::ensure!(!endpoints.is_empty() && endpoints.len() <= 512, "Provide 1 to 512 endpoints");
    let base = reqwest::Url::parse(base_url)?;
    anyhow::ensure!(matches!(base.scheme(), "http" | "https") && base.host_str().is_some(), "An absolute HTTP(S) base URL is required");
    anyhow::ensure!(base.username().is_empty() && base.password().is_none() && base.query().is_none() && base.fragment().is_none() && base.path() == "/", "Base URL must be an origin without credentials, path, query or fragment");
    anyhow::ensure!(base_url.len() <= 8192 && !base_url.chars().any(char::is_control), "Base URL is invalid or oversized");
    let origin = base.as_str().trim_end_matches('/');
    let mut output = String::from("# Generated from ASP.NET source. Review route values, authentication and request bodies before sending.\n\n");
    let mut count = 0;
    for endpoint in endpoints {
        let method = endpoint.method.as_str();
        if !METHODS.contains(&method) { continue; }
        let route = sample_route(&endpoint.route)?;
        let url = reqwest::Url::parse(&format!("{origin}{route}"))?;
        anyhow::ensure!(url.origin() == base.origin() && url.query().is_none() && url.fragment().is_none(), "Endpoint route changes the request origin");
        let label = endpoint.handler.chars().take(120).map(|c| if c.is_control() { ' ' } else { c }).collect::<String>();
        output.push_str(&format!("### {method} {label}\n{method} {url}\nAccept: application/json\n\n"));
        count += 1;
        anyhow::ensure!(output.len() <= 1024 * 1024, "Generated HTTP file exceeds 1 MiB");
    }
    anyhow::ensure!(count > 0, "No endpoint has an explicit supported HTTP method");
    Ok(output)
}

fn sample_route(route: &str) -> anyhow::Result<String> {
    anyhow::ensure!(route.len() <= 8192 && route.starts_with('/') && !route.contains('#') && !route.chars().any(|c| c.is_control() || c.is_whitespace()), "Endpoint route is invalid or oversized");
    let mut rendered = String::new();
    let mut chars = route.chars();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                let mut parameter = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some('{') | None => anyhow::bail!("Endpoint route has an invalid parameter"),
                        Some(c) => parameter.push(c),
                    }
                }
                let parameter = parameter.trim_start_matches('*');
                let name = parameter.split([':', '?', '=']).next().unwrap_or("");
                anyhow::ensure!(!name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'), "Endpoint route has an invalid parameter name");
                let constraint = parameter.split_once(':').map(|(_, c)| c.split(['?', '=']).next().unwrap_or("")).unwrap_or("");
                let sample = match constraint.to_ascii_lowercase().as_str() {
                    "int" | "long" | "decimal" | "double" | "float" => "1",
                    "guid" => "00000000-0000-0000-0000-000000000001",
                    "bool" => "true",
                    _ => "sample",
                };
                rendered.push_str(sample);
            }
            '}' => anyhow::bail!("Endpoint route has an unmatched closing brace"),
            '?' => anyhow::bail!("Endpoint route cannot contain a query string"),
            _ => rendered.push(c),
        }
    }
    anyhow::ensure!(!rendered.starts_with("//"), "Endpoint route cannot be a network-path reference");
    Ok(rendered)
}
