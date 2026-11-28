//! The app host's resource service: the gRPC `aspire.v1.DashboardService` the Aspire dashboard
//! uses. `WatchResources` streams every resource (projects, containers, executables) with its
//! state, URLs, properties and commands; `WatchResourceConsoleLogs` streams one resource's
//! output; `ExecuteResourceCommand` runs a command (start, stop, restart…).
//!
//! gRPC goes over HTTP/2 (TLS with the ASP.NET development certificate, or plain text when the
//! app host allows unsecured transport) with the API key the IDE gave the app host.

use super::proto::{Frames, Reader, Writer, frame, value_text};
use rc_core::Runtime;
use rc_core::inbox::Outbox;
use std::time::Duration;

const SERVICE: &str = "aspire.v1.DashboardService";
const API_KEY_HEADER: &str = "x-resource-service-api-key";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceUrl {
    pub name: Option<String>,
    pub url: String,
    pub display_name: Option<String>,
    pub internal: bool,
    pub inactive: bool,
    pub sort_order: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceCommand {
    /// `resource-start`, `resource-stop`, `resource-restart` or one the app defines.
    pub name: String,
    pub display_name: String,
    pub confirmation: Option<String>,
    pub enabled: bool,
    pub hidden: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceProperty {
    pub name: String,
    pub display_name: Option<String>,
    pub value: String,
    pub sensitive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Healthy,
    Unhealthy,
    Degraded,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resource {
    /// Unique instance name (`apiservice-abcd1234`): what commands and logs take.
    pub name: String,
    /// `Project`, `Container`, `Executable`, `Parameter`…
    pub resource_type: String,
    /// Name in the app model (`apiservice`).
    pub display_name: String,
    pub uid: String,
    /// `Starting`, `Running`, `Finished`, `Exited`, `FailedToStart`, `Waiting`…
    pub state: Option<String>,
    /// `success`, `info`, `warning` or `error`.
    pub state_style: Option<String>,
    pub urls: Vec<ResourceUrl>,
    pub commands: Vec<ResourceCommand>,
    pub properties: Vec<ResourceProperty>,
    /// Combined health checks, when the resource has any.
    pub health: Option<Health>,
    pub hidden: bool,
}

impl Resource {
    pub fn property(&self, name: &str) -> Option<&str> {
        self.properties.iter().find(|p| p.name == name).map(|p| p.value.as_str())
    }

    /// The project file of a project resource.
    pub fn project_path(&self) -> Option<&str> {
        self.property("project.path")
    }

    /// The command, if the resource offers it and it is not hidden.
    pub fn command(&self, name: &str) -> Option<&ResourceCommand> {
        self.commands.iter().find(|c| c.name == name && !c.hidden)
    }

    /// URLs to show (external first, by the order the app host asks for).
    pub fn visible_urls(&self) -> Vec<&ResourceUrl> {
        let mut urls: Vec<&ResourceUrl> = self.urls.iter().filter(|u| !u.inactive && !u.url.is_empty()).collect();
        urls.sort_by_key(|u| (u.internal, -u.sort_order));
        urls
    }

    fn decode(buf: &[u8]) -> Resource {
        let mut r = Resource::default();
        let mut health: Vec<Option<u64>> = Vec::new();
        for (field, v) in Reader::new(buf) {
            match field {
                1 => r.name = v.as_str(),
                2 => r.resource_type = v.as_str(),
                3 => r.display_name = v.as_str(),
                4 => r.uid = v.as_str(),
                5 => r.state = Some(v.as_str()).filter(|s| !s.is_empty()),
                11 => r.commands.push(decode_command(v.as_bytes())),
                12 => r.properties.push(decode_property(v.as_bytes())),
                13 => r.urls.push(decode_url(v.as_bytes())),
                14 => r.state_style = Some(v.as_str()).filter(|s| !s.is_empty()),
                17 => health.push(Reader::new(v.as_bytes()).find(|(f, _)| *f == 1).map(|(_, s)| s.as_u64())),
                21 => r.hidden = v.as_bool(),
                _ => {}
            }
        }
        // No status yet means a check has not run: not healthy.
        if !health.is_empty() {
            r.health = Some(if health.iter().all(|s| *s == Some(0)) {
                Health::Healthy
            } else if health.iter().any(|s| *s == Some(1) || s.is_none()) {
                Health::Unhealthy
            } else {
                Health::Degraded
            });
        }
        if r.display_name.is_empty() {
            r.display_name = r.name.clone();
        }
        r
    }
}

fn decode_command(buf: &[u8]) -> ResourceCommand {
    let mut c = ResourceCommand { enabled: true, ..Default::default() };
    for (field, v) in Reader::new(buf) {
        match field {
            1 => c.name = v.as_str(),
            2 => c.display_name = v.as_str(),
            3 => c.confirmation = Some(v.as_str()).filter(|s| !s.is_empty()),
            9 => {
                c.enabled = v.as_u64() == 0;
                c.hidden = v.as_u64() == 2;
            }
            _ => {}
        }
    }
    c
}

fn decode_property(buf: &[u8]) -> ResourceProperty {
    let mut p = ResourceProperty::default();
    for (field, v) in Reader::new(buf) {
        match field {
            1 => p.name = v.as_str(),
            2 => p.display_name = Some(v.as_str()).filter(|s| !s.is_empty()),
            3 => p.value = value_text(v.as_bytes()),
            4 => p.sensitive = v.as_bool(),
            _ => {}
        }
    }
    p
}

fn decode_url(buf: &[u8]) -> ResourceUrl {
    let mut u = ResourceUrl::default();
    for (field, v) in Reader::new(buf) {
        match field {
            1 => u.name = Some(v.as_str()),
            2 => u.url = v.as_str(),
            3 => u.internal = v.as_bool(),
            4 => u.inactive = v.as_bool(),
            5 => {
                for (f, d) in Reader::new(v.as_bytes()) {
                    match f {
                        1 => u.sort_order = d.as_u64() as i32 as i64,
                        2 => u.display_name = Some(d.as_str()).filter(|s| !s.is_empty()),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    u
}

/// One line of a resource's console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub text: String,
    pub stderr: bool,
}

/// What the watches report.
#[derive(Clone, Debug, PartialEq)]
pub enum ResourceEvent {
    /// The service answered: every resource (replaces what was known).
    Snapshot(Vec<Resource>),
    Upsert(Resource),
    Deleted(String),
    /// The stream ended or could not start (the app host may still be starting).
    Disconnected(String),
    Logs { resource: String, lines: Vec<LogLine> },
    /// A command finished: `Ok(message)` or the error.
    CommandDone { resource: String, command: String, result: Result<Option<String>, String> },
}

/// Decodes a `WatchResourcesUpdate` message.
pub fn decode_update(buf: &[u8]) -> Vec<ResourceEvent> {
    let mut out = Vec::new();
    for (field, v) in Reader::new(buf) {
        match field {
            1 => out.push(ResourceEvent::Snapshot(
                Reader::new(v.as_bytes()).filter(|(f, _)| *f == 1).map(|(_, r)| Resource::decode(r.as_bytes())).collect(),
            )),
            2 => {
                for (_, change) in Reader::new(v.as_bytes()).filter(|(f, _)| *f == 1) {
                    for (kind, body) in Reader::new(change.as_bytes()) {
                        match kind {
                            1 => {
                                let name = Reader::new(body.as_bytes()).find(|(f, _)| *f == 1).map(|(_, n)| n.as_str()).unwrap_or_default();
                                out.push(ResourceEvent::Deleted(name));
                            }
                            2 => out.push(ResourceEvent::Upsert(Resource::decode(body.as_bytes()))),
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn decode_log_lines(buf: &[u8]) -> Vec<LogLine> {
    Reader::new(buf)
        .filter(|(f, _)| *f == 1)
        .map(|(_, line)| {
            let mut l = LogLine { text: String::new(), stderr: false };
            for (f, v) in Reader::new(line.as_bytes()) {
                match f {
                    1 => l.text = v.as_str(),
                    2 => l.stderr = v.as_bool(),
                    _ => {}
                }
            }
            l
        })
        .collect()
}

/// Where the resource service listens and the key it expects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceEndpoint {
    pub url: String,
    pub api_key: String,
}

/// A client of one app host's resource service.
#[derive(Clone)]
pub struct ResourceService {
    endpoint: ResourceEndpoint,
    client: reqwest::Client,
}

impl ResourceService {
    pub fn new(endpoint: ResourceEndpoint) -> anyhow::Result<Self> {
        // The service only listens on loopback, with the development certificate.
        let client = reqwest::Client::builder()
            .http2_prior_knowledge()
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .build()?;
        Ok(ResourceService { endpoint, client })
    }

    pub fn endpoint(&self) -> &ResourceEndpoint {
        &self.endpoint
    }

    async fn call(&self, method: &str, message: &[u8]) -> anyhow::Result<reqwest::Response> {
        let url = format!("{}/{SERVICE}/{method}", self.endpoint.url.trim_end_matches('/'));
        let response = self
            .client
            .post(url)
            .header("content-type", "application/grpc")
            .header("te", "trailers")
            .header(API_KEY_HEADER, &self.endpoint.api_key)
            .body(frame(message))
            .send()
            .await?;
        anyhow::ensure!(response.status().is_success(), "HTTP {}", response.status());
        // A failed call without a body carries its status in the headers ("trailers-only").
        if let Some(status) = response.headers().get("grpc-status").and_then(|s| s.to_str().ok())
            && status != "0"
        {
            let message = response.headers().get("grpc-message").and_then(|m| m.to_str().ok()).unwrap_or("").to_string();
            anyhow::bail!("gRPC status {status} {}", percent_decode(&message));
        }
        Ok(response)
    }

    /// Streams the resources until the handle is dropped, reconnecting while the app host
    /// starts or after the stream ends.
    pub fn watch(&self, out: Outbox<ResourceEvent>) -> WatchHandle {
        let service = self.clone();
        WatchHandle(Runtime::spawn(async move {
            let mut reconnect = false;
            loop {
                let mut request = Writer::default();
                if reconnect {
                    request.bool(1, true);
                }
                match service.call("WatchResources", &request.0).await {
                    Ok(mut response) => {
                        reconnect = true;
                        let mut frames = Frames::default();
                        loop {
                            match response.chunk().await {
                                Ok(Some(chunk)) => {
                                    for message in frames.push(&chunk) {
                                        for event in decode_update(&message) {
                                            if !out.send(event) {
                                                return;
                                            }
                                        }
                                    }
                                }
                                Ok(None) => {
                                    out.send(ResourceEvent::Disconnected(String::new()));
                                    break;
                                }
                                Err(error) => {
                                    out.send(ResourceEvent::Disconnected(error.to_string()));
                                    break;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        if !out.send(ResourceEvent::Disconnected(error.to_string())) {
                            return;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(700)).await;
            }
        }))
    }

    /// Streams one resource's console (what it printed so far, then new lines).
    pub fn watch_logs(&self, resource: &str, out: Outbox<ResourceEvent>) -> WatchHandle {
        let service = self.clone();
        let resource = resource.to_string();
        WatchHandle(Runtime::spawn(async move {
            let mut request = Writer::default();
            request.string(1, &resource);
            let mut response = match service.call("WatchResourceConsoleLogs", &request.0).await {
                Ok(response) => response,
                Err(error) => {
                    out.send(ResourceEvent::Disconnected(error.to_string()));
                    return;
                }
            };
            let mut frames = Frames::default();
            while let Ok(Some(chunk)) = response.chunk().await {
                for message in frames.push(&chunk) {
                    let lines = decode_log_lines(&message);
                    if !lines.is_empty() && !out.send(ResourceEvent::Logs { resource: resource.clone(), lines }) {
                        return;
                    }
                }
            }
        }))
    }

    /// Runs a command of a resource; the result arrives as [`ResourceEvent::CommandDone`].
    pub fn execute(&self, resource: &Resource, command: &str, out: Outbox<ResourceEvent>) {
        let service = self.clone();
        let (name, kind, command) = (resource.name.clone(), resource.resource_type.clone(), command.to_string());
        Runtime::spawn(async move {
            let mut request = Writer::default();
            request.string(1, &command).string(2, &name).string(3, &kind).bool(6, true);
            let result = async {
                let response = service.call("ExecuteResourceCommand", &request.0).await?;
                let body = response.bytes().await?;
                let message = Frames::default().push(&body).into_iter().next().ok_or_else(|| anyhow::anyhow!("no response"))?;
                Ok::<_, anyhow::Error>(message)
            }
            .await;
            let result = match result {
                Err(error) => Err(error.to_string()),
                Ok(message) => {
                    let (mut kind, mut text) = (0, None);
                    for (field, v) in Reader::new(&message) {
                        match field {
                            1 => kind = v.as_u64(),
                            2 | 3 if !v.as_str().is_empty() => text = Some(v.as_str()),
                            _ => {}
                        }
                    }
                    match kind {
                        1 => Ok(text),
                        3 => Err(text.unwrap_or_else(|| "cancelled".into())),
                        _ => Err(text.unwrap_or_else(|| "the command failed".into())),
                    }
                }
            };
            out.send(ResourceEvent::CommandDone { resource: name, command, result });
        });
    }
}

/// Stops a stream when dropped.
pub struct WatchHandle(tokio::task::JoinHandle<()>);

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(name: &str, state: &str) -> Vec<u8> {
        let mut url = Writer::default();
        url.string(1, "https").string(2, "https://localhost:7426");
        let mut internal = Writer::default();
        internal.string(2, "http://localhost:5500").bool(3, true);
        let mut command = Writer::default();
        command.string(1, "resource-stop").string(2, "Stop");
        let mut hidden = Writer::default();
        hidden.string(1, "resource-start").string(2, "Start").0.extend([9 << 3, 2]);
        let mut value = Writer::default();
        value.string(3, "/w/Api/Api.csproj");
        let mut property = Writer::default();
        property.string(1, "project.path").bytes(3, &value.0);
        let mut health = Writer::default();
        health.0.extend([1 << 3, 0]);
        health.string(2, "health");
        let mut r = Writer::default();
        r.string(1, name).string(2, "Project").string(3, "apiservice").string(5, state).bytes(11, &command.0).bytes(11, &hidden.0).bytes(12, &property.0).bytes(13, &internal.0).bytes(13, &url.0).bytes(17, &health.0);
        r.0
    }

    #[test]
    fn decodes_the_initial_snapshot_and_changes() {
        let mut initial = Writer::default();
        initial.bytes(1, &resource("apiservice-x1", "Running"));
        let mut update = Writer::default();
        update.bytes(1, &initial.0);
        let events = decode_update(&update.0);
        let [ResourceEvent::Snapshot(list)] = events.as_slice() else { panic!("{events:?}") };
        let r = &list[0];
        assert_eq!((r.name.as_str(), r.display_name.as_str(), r.resource_type.as_str()), ("apiservice-x1", "apiservice", "Project"));
        assert_eq!(r.state.as_deref(), Some("Running"));
        assert_eq!(r.project_path(), Some("/w/Api/Api.csproj"));
        assert_eq!(r.health, Some(Health::Healthy));
        assert!(r.command("resource-stop").is_some_and(|c| c.enabled));
        assert!(r.command("resource-start").is_none(), "hidden commands are left out");
        assert_eq!(r.visible_urls().iter().map(|u| u.url.as_str()).collect::<Vec<_>>(), ["https://localhost:7426", "http://localhost:5500"]);

        let mut deletion = Writer::default();
        deletion.string(1, "cache-y2");
        let mut upsert = Writer::default();
        upsert.bytes(2, &resource("apiservice-x1", "Finished"));
        let mut delete = Writer::default();
        delete.bytes(1, &deletion.0);
        let mut changes = Writer::default();
        changes.bytes(1, &upsert.0).bytes(1, &delete.0);
        let mut update = Writer::default();
        update.bytes(2, &changes.0);
        let events = decode_update(&update.0);
        assert!(matches!(&events[0], ResourceEvent::Upsert(r) if r.state.as_deref() == Some("Finished")));
        assert_eq!(events[1], ResourceEvent::Deleted("cache-y2".into()));
    }

    #[test]
    fn decodes_console_lines() {
        let mut line = Writer::default();
        line.string(1, "info: started").bool(2, true);
        let mut update = Writer::default();
        update.bytes(1, &line.0);
        assert_eq!(decode_log_lines(&update.0), vec![LogLine { text: "info: started".into(), stderr: true }]);
    }

    #[test]
    fn grpc_messages_are_percent_decoded() {
        assert_eq!(percent_decode("Bad%20key%3A"), "Bad key:");
    }
}
