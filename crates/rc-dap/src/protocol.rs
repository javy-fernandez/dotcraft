//! The Debug Adapter Protocol on the wire: `Content-Length` framing ([`frame`],
//! [`FrameDecoder`]) and the message bodies DotCraft uses, as serde types.
//!
//! Every type tolerates missing optional fields: adapters differ in what they send
//! (netcoredbg, for example, never sends `hitBreakpointIds`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// Frames a JSON body with its `Content-Length` header (the length counts bytes).
pub fn frame(body: &str) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

/// Splits a byte stream into framed JSON messages. Feed it whatever the pipe returns
/// ([`FrameDecoder::push`]) and take complete messages with [`FrameDecoder::next_message`].
#[derive(Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

/// A frame whose body is not valid JSON (skipped, reported for the logs).
#[derive(Debug, Clone, PartialEq)]
pub struct BadFrame(pub String);

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete message: `None` until one has fully arrived.
    pub fn next_message(&mut self) -> Option<Result<Value, BadFrame>> {
        loop {
            let header_end = find(&self.buf, b"\r\n\r\n")?;
            let header = String::from_utf8_lossy(&self.buf[..header_end]).into_owned();
            let len = header.lines().find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.trim().eq_ignore_ascii_case("Content-Length").then(|| v.trim().parse::<usize>().ok()).flatten()
            });
            let body_start = header_end + 4;
            let Some(len) = len else {
                // A header without length: drop it and resynchronise on the next one.
                self.buf.drain(..body_start);
                continue;
            };
            if self.buf.len() < body_start + len {
                return None;
            }
            let body: Vec<u8> = self.buf.drain(..body_start + len).skip(body_start).collect();
            return Some(serde_json::from_slice(&body).map_err(|e| BadFrame(format!("{e}: {}", String::from_utf8_lossy(&body)))));
        }
    }

    /// Bytes received but not yet part of a complete message.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

// ---------------------------------------------------------------------------------------------
// Requests and responses.

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExceptionBreakpointsFilter {
    pub filter: String,
    pub label: String,
    pub description: Option<String>,
    /// Whether the filter should be on when the user never chose.
    #[serde(rename = "default")]
    pub default_on: bool,
    pub supports_condition: bool,
}

/// What the adapter supports (`initialize` response).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Capabilities {
    pub supports_configuration_done_request: bool,
    pub supports_function_breakpoints: bool,
    pub supports_conditional_breakpoints: bool,
    pub supports_hit_conditional_breakpoints: bool,
    pub supports_evaluate_for_hovers: bool,
    pub exception_breakpoint_filters: Vec<ExceptionBreakpointsFilter>,
    pub supports_set_variable: bool,
    pub supports_set_expression: bool,
    pub supports_log_points: bool,
    pub supports_exception_info_request: bool,
    pub supports_exception_filter_options: bool,
    pub supports_terminate_request: bool,
    pub support_terminate_debuggee: bool,
    pub supports_cancel_request: bool,
    pub supports_delayed_stack_trace_loading: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Source {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_reference: Option<i64>,
}

/// A breakpoint as sent in `setBreakpoints` (1-based line).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBreakpoint {
    pub line: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hit_condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_message: Option<String>,
}

/// A breakpoint as the adapter reports it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Breakpoint {
    pub id: Option<i64>,
    pub verified: bool,
    pub message: Option<String>,
    pub source: Option<Source>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub end_line: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SetBreakpointsResponse {
    pub breakpoints: Vec<Breakpoint>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Thread {
    pub id: i64,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadsResponse {
    pub threads: Vec<Thread>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StackFrame {
    pub id: i64,
    pub name: String,
    pub source: Option<Source>,
    /// 1-based; 0 when the frame has no source.
    pub line: u32,
    pub column: u32,
    pub end_line: Option<u32>,
    pub end_column: Option<u32>,
    pub module_id: Option<Value>,
    pub presentation_hint: Option<String>,
}

impl StackFrame {
    /// Source file of the frame, when it has one on disk.
    pub fn path(&self) -> Option<&std::path::Path> {
        self.source.as_ref()?.path.as_deref()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StackTraceResponse {
    pub stack_frames: Vec<StackFrame>,
    pub total_frames: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Scope {
    pub name: String,
    pub presentation_hint: Option<String>,
    pub variables_reference: i64,
    pub named_variables: Option<i64>,
    pub indexed_variables: Option<i64>,
    pub expensive: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScopesResponse {
    pub scopes: Vec<Scope>,
}

/// A variable; `variables_reference > 0` means it has children (expand it).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Variable {
    pub name: String,
    pub value: String,
    #[serde(rename = "type")]
    pub type_name: Option<String>,
    pub evaluate_name: Option<String>,
    pub variables_reference: i64,
    pub named_variables: Option<i64>,
    pub indexed_variables: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VariablesResponse {
    pub variables: Vec<Variable>,
}

/// Result of `evaluate`; expandable when `variables_reference > 0`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EvaluateResponse {
    pub result: String,
    #[serde(rename = "type")]
    pub type_name: Option<String>,
    pub variables_reference: i64,
    pub named_variables: Option<i64>,
    pub indexed_variables: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SetVariableResponse {
    pub value: String,
    #[serde(rename = "type")]
    pub type_name: Option<String>,
    pub variables_reference: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExceptionDetails {
    pub message: Option<String>,
    pub type_name: Option<String>,
    pub full_type_name: Option<String>,
    pub evaluate_name: Option<String>,
    pub stack_trace: Option<String>,
    pub inner_exception: Vec<ExceptionDetails>,
}

/// `exceptionInfo` response: what was thrown where the program stopped.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExceptionInfo {
    pub exception_id: String,
    pub description: Option<String>,
    /// `never`, `always`, `unhandled`, `userUnhandled`.
    pub break_mode: String,
    pub details: Option<ExceptionDetails>,
}

/// Where an expression is evaluated (changes side-effect rules and formatting).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EvalContext {
    Watch,
    Repl,
    Hover,
}

impl EvalContext {
    pub fn as_str(self) -> &'static str {
        match self {
            EvalContext::Watch => "watch",
            EvalContext::Repl => "repl",
            EvalContext::Hover => "hover",
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Events.

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoppedEvent {
    /// `breakpoint`, `step`, `exception`, `pause`, `entry`, `goto`, `function breakpoint`…
    pub reason: String,
    pub description: Option<String>,
    pub thread_id: Option<i64>,
    pub all_threads_stopped: bool,
    pub hit_breakpoint_ids: Vec<i64>,
    pub text: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContinuedEvent {
    pub thread_id: i64,
    pub all_threads_continued: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExitedEvent {
    pub exit_code: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OutputEvent {
    pub category: Option<String>,
    pub output: String,
    pub source: Option<Source>,
    pub line: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BreakpointEvent {
    /// `changed`, `new`, `removed`.
    pub reason: String,
    pub breakpoint: Breakpoint,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadEvent {
    /// `started` or `exited`.
    pub reason: String,
    pub thread_id: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Module {
    pub id: Value,
    pub name: String,
    pub path: Option<PathBuf>,
    pub symbol_status: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModuleEvent {
    /// `new`, `changed`, `removed`.
    pub reason: String,
    pub module: Module,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProcessEvent {
    pub name: String,
    pub system_process_id: Option<u32>,
    pub start_method: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frames_count_bytes_not_chars() {
        let body = json!({"output": "señal ✓"}).to_string();
        let f = frame(&body);
        let text = String::from_utf8(f).unwrap();
        assert!(text.starts_with(&format!("Content-Length: {}\r\n\r\n", body.len())));
        assert!(body.len() > body.chars().count());
    }

    #[test]
    fn decoder_handles_split_and_joined_frames() {
        let a = json!({"seq": 1, "type": "event", "event": "output", "body": {"output": "ñandú\n"}}).to_string();
        let b = json!({"seq": 2, "type": "event", "event": "initialized"}).to_string();
        let mut bytes = frame(&a);
        bytes.extend(frame(&b));
        // Byte by byte: nothing until the first body is complete, cutting UTF-8 sequences.
        let mut d = FrameDecoder::new();
        let mut got = Vec::new();
        for byte in &bytes {
            d.push(std::slice::from_ref(byte));
            while let Some(m) = d.next_message() {
                got.push(m.unwrap());
            }
        }
        assert_eq!(got.len(), 2);
        assert_eq!(got[0]["body"]["output"], "ñandú\n");
        assert_eq!(got[1]["event"], "initialized");
        assert_eq!(d.buffered(), 0);

        // All at once, plus extra headers and a garbage frame in between.
        let mut d = FrameDecoder::new();
        d.push(b"Content-Type: application/vscode-jsonrpc\r\ncontent-length: 2\r\n\r\n{}");
        d.push(&frame("not json"));
        d.push(b"X-Nothing: 1\r\n\r\n");
        d.push(&frame(&b));
        assert_eq!(d.next_message().unwrap().unwrap(), json!({}));
        assert!(d.next_message().unwrap().is_err());
        assert_eq!(d.next_message().unwrap().unwrap()["seq"], 2);
        assert!(d.next_message().is_none());
    }

    #[test]
    fn parses_netcoredbg_bodies() {
        let caps: Capabilities = serde_json::from_value(json!({
            "exceptionBreakpointFilters": [{"filter": "user-unhandled", "label": "user-unhandled"}, {"filter": "all", "label": "all"}],
            "supportsConditionalBreakpoints": true, "supportsExceptionInfoRequest": true, "supportsSetVariable": true
        }))
        .unwrap();
        assert_eq!(caps.exception_breakpoint_filters[1].filter, "all");
        assert!(!caps.supports_log_points);
        let v: Variable = serde_json::from_value(json!({"evaluateName": "order", "name": "order", "namedVariables": 5, "type": "Shop.Core.Order", "value": "{Shop.Core.Order}", "variablesReference": 3})).unwrap();
        assert_eq!((v.type_name.as_deref(), v.variables_reference), (Some("Shop.Core.Order"), 3));
        let s: StoppedEvent = serde_json::from_value(json!({"allThreadsStopped": true, "reason": "breakpoint", "threadId": 41219577})).unwrap();
        assert_eq!(s.thread_id, Some(41219577));
        assert!(s.hit_breakpoint_ids.is_empty());
        let sb = serde_json::to_value(SourceBreakpoint { line: 3, condition: Some("x > 1".into()), ..Default::default() }).unwrap();
        assert_eq!(sb, json!({"line": 3, "condition": "x > 1"}));
    }
}
