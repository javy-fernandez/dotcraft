//! Source breakpoints by file and 1-based line, with what the debugger made of them
//! (verified, bound line, message), plus the client-side parts adapters may lack: hit
//! conditions and log messages (netcoredbg supports neither natively).

use crate::protocol::{Breakpoint, SourceBreakpoint};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A breakpoint as the user set it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BreakpointSpec {
    pub path: PathBuf,
    /// 1-based line.
    pub line: u32,
    /// Stop only when this expression is true.
    pub condition: Option<String>,
    /// `5`, `>= 5`, `== 5`, `% 5`…: stop on the hits that satisfy it.
    pub hit_condition: Option<String>,
    /// A logpoint: print this (with `{expression}` parts) instead of stopping.
    pub log_message: Option<String>,
    pub enabled: bool,
}

impl BreakpointSpec {
    pub fn new(path: impl Into<PathBuf>, line: u32) -> Self {
        BreakpointSpec { path: path.into(), line, condition: None, hit_condition: None, log_message: None, enabled: true }
    }

    pub fn with_condition(mut self, condition: impl Into<String>) -> Self {
        self.condition = Some(condition.into()).filter(|c: &String| !c.trim().is_empty());
        self
    }

    pub fn with_hit_condition(mut self, hit: impl Into<String>) -> Self {
        self.hit_condition = Some(hit.into()).filter(|c: &String| !c.trim().is_empty());
        self
    }

    pub fn with_log_message(mut self, message: impl Into<String>) -> Self {
        self.log_message = Some(message.into()).filter(|c: &String| !c.is_empty());
        self
    }
}

/// A breakpoint and its state in the running session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BreakpointState {
    pub spec: BreakpointSpec,
    /// The debugger resolved it to code (draw it filled).
    pub verified: bool,
    /// Line the debugger bound it to (may differ from `spec.line`); `None` until known.
    pub bound_line: Option<u32>,
    /// Why it is not verified, from the debugger.
    pub message: Option<String>,
    /// Id the adapter gave it.
    pub adapter_id: Option<i64>,
    /// Times it was hit in this session (counts only stops that matched it).
    pub hits: u32,
}

impl BreakpointState {
    fn new(spec: BreakpointSpec) -> Self {
        BreakpointState { spec, verified: false, bound_line: None, message: None, adapter_id: None, hits: 0 }
    }

    /// The line to draw it on: the bound line when known.
    pub fn display_line(&self) -> u32 {
        self.bound_line.unwrap_or(self.spec.line)
    }

    fn apply(&mut self, bp: &Breakpoint) {
        self.verified = bp.verified;
        if bp.id.is_some() {
            self.adapter_id = bp.id;
        }
        if let Some(line) = bp.line {
            self.bound_line = Some(line);
        }
        self.message = bp.message.clone().filter(|_| !bp.verified);
    }
}

/// Which client-side emulations the adapter needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Emulation {
    pub hit_conditions: bool,
    pub log_points: bool,
}

/// All breakpoints, grouped by file.
#[derive(Clone, Debug, Default)]
pub struct Breakpoints {
    files: BTreeMap<PathBuf, Vec<BreakpointState>>,
}

impl Breakpoints {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces the breakpoint at `spec.path:spec.line`.
    pub fn set(&mut self, spec: BreakpointSpec) {
        let list = self.files.entry(spec.path.clone()).or_default();
        match list.iter_mut().find(|b| b.spec.line == spec.line) {
            Some(b) => {
                if b.spec.enabled != spec.enabled || b.spec.condition != spec.condition || b.spec.hit_condition != spec.hit_condition {
                    b.hits = 0;
                }
                b.spec = spec;
            }
            None => {
                list.push(BreakpointState::new(spec));
                list.sort_by_key(|b| b.spec.line);
            }
        }
    }

    /// Removes it; false when there was none.
    pub fn remove(&mut self, path: &Path, line: u32) -> bool {
        let Some(list) = self.files.get_mut(path) else { return false };
        let before = list.len();
        list.retain(|b| b.spec.line != line);
        let removed = list.len() != before;
        if list.is_empty() {
            self.files.remove(path);
        }
        removed
    }

    pub fn get(&self, path: &Path, line: u32) -> Option<&BreakpointState> {
        self.files.get(path)?.iter().find(|b| b.spec.line == line)
    }

    pub fn get_mut(&mut self, path: &Path, line: u32) -> Option<&mut BreakpointState> {
        self.files.get_mut(path)?.iter_mut().find(|b| b.spec.line == line)
    }

    pub fn in_file(&self, path: &Path) -> &[BreakpointState] {
        self.files.get(path).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn files(&self) -> impl Iterator<Item = &Path> {
        self.files.keys().map(PathBuf::as_path)
    }

    pub fn iter(&self) -> impl Iterator<Item = &BreakpointState> {
        self.files.values().flatten()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// What `setBreakpoints` sends for a file: the enabled breakpoints, in order, with the
    /// lines they belong to.
    pub fn request_for(&self, path: &Path, emulation: Emulation) -> (Vec<u32>, Vec<SourceBreakpoint>) {
        self.in_file(path)
            .iter()
            .filter(|b| b.spec.enabled)
            .map(|b| {
                let s = &b.spec;
                let sb = SourceBreakpoint {
                    line: s.line,
                    column: None,
                    condition: s.condition.clone(),
                    hit_condition: s.hit_condition.clone().filter(|_| !emulation.hit_conditions),
                    log_message: s.log_message.clone().filter(|_| !emulation.log_points),
                };
                (s.line, sb)
            })
            .unzip()
    }

    /// Applies the `setBreakpoints` response (same order as the request's `lines`).
    pub fn apply_response(&mut self, path: &Path, lines: &[u32], response: &[Breakpoint]) {
        let Some(list) = self.files.get_mut(path) else { return };
        for (line, bp) in lines.iter().zip(response) {
            if let Some(b) = list.iter_mut().find(|b| b.spec.line == *line) {
                b.apply(bp);
            }
        }
        // Disabled ones are not in the debugger.
        for b in list.iter_mut().filter(|b| !b.spec.enabled) {
            b.verified = false;
            b.adapter_id = None;
            b.bound_line = None;
        }
    }

    /// Applies a `breakpoint` event; returns the file it belongs to.
    pub fn apply_event(&mut self, bp: &Breakpoint) -> Option<PathBuf> {
        let id = bp.id?;
        for (path, list) in &mut self.files {
            if let Some(b) = list.iter_mut().find(|b| b.adapter_id == Some(id)) {
                b.apply(bp);
                return Some(path.clone());
            }
        }
        None
    }

    /// Breakpoints at a stop location: by adapter id when the adapter says which, otherwise
    /// the enabled breakpoints bound to that file and line.
    pub fn at_stop(&self, hit_ids: &[i64], path: Option<&Path>, line: u32) -> Vec<(PathBuf, u32)> {
        if !hit_ids.is_empty() {
            return self.iter().filter(|b| b.adapter_id.is_some_and(|id| hit_ids.contains(&id))).map(|b| (b.spec.path.clone(), b.spec.line)).collect();
        }
        let Some(path) = path else { return Vec::new() };
        self.files
            .iter()
            .filter(|(p, _)| same_file(p, path))
            .flat_map(|(_, l)| l)
            .filter(|b| b.spec.enabled && b.display_line() == line)
            .map(|b| (b.spec.path.clone(), b.spec.line))
            .collect()
    }

    pub fn reset_hits(&mut self) {
        for b in self.files.values_mut().flatten() {
            b.hits = 0;
        }
    }

    /// Forgets what the debugger said (a new session starts).
    pub fn reset_state(&mut self) {
        for b in self.files.values_mut().flatten() {
            *b = BreakpointState::new(b.spec.clone());
        }
    }
}

/// Paths compared as the OS does (case-insensitive on macOS/Windows file systems, through
/// symlinks such as `/tmp` → `/private/tmp`).
pub fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    if let (Ok(ca), Ok(cb)) = (a.canonicalize(), b.canonicalize()) {
        return ca == cb;
    }
    if cfg!(any(windows, target_os = "macos")) { a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()) } else { false }
}

/// Whether hit number `hits` (1-based) satisfies a VS-style hit condition: `5` / `== 5`
/// (exactly the 5th), `>= 5`, `> 5`, `< 5`, `<= 5`, `% 5` (every 5th). Unparseable
/// conditions always stop, so a typo never makes a breakpoint silently dead.
pub fn hit_condition_met(condition: &str, hits: u32) -> bool {
    let c = condition.trim();
    let (op, rest) = ["==", ">=", "<=", ">", "<", "%", "="].iter().find_map(|op| c.strip_prefix(op).map(|r| (*op, r))).unwrap_or(("==", c));
    let Ok(n) = rest.trim().parse::<u32>() else { return true };
    match op {
        "==" | "=" => hits == n,
        ">=" => hits >= n,
        "<=" => hits <= n,
        ">" => hits > n,
        "<" => hits < n,
        "%" => n != 0 && hits.is_multiple_of(n),
        _ => true,
    }
}

/// A logpoint message split into literal text and `{expression}` parts (`{{`/`}}` escape braces).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogPart {
    Text(String),
    Expr(String),
}

pub fn parse_log_message(message: &str) -> Vec<LogPart> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut chars = message.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                text.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                text.push('}');
            }
            '{' => {
                let mut expr = String::new();
                let mut depth = 0;
                for c in chars.by_ref() {
                    match c {
                        '{' => depth += 1,
                        '}' if depth == 0 => break,
                        '}' => depth -= 1,
                        _ => {}
                    }
                    expr.push(c);
                }
                if !text.is_empty() {
                    parts.push(LogPart::Text(std::mem::take(&mut text)));
                }
                parts.push(LogPart::Expr(expr.trim().to_string()));
            }
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        parts.push(LogPart::Text(text));
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_conditions() {
        assert!(hit_condition_met("3", 3));
        assert!(!hit_condition_met("3", 2));
        assert!(hit_condition_met("== 3", 3));
        assert!(hit_condition_met(">= 2", 5));
        assert!(!hit_condition_met("> 5", 5));
        assert!(hit_condition_met("<3", 2));
        assert!(hit_condition_met("% 2", 4));
        assert!(!hit_condition_met("% 2", 3));
        assert!(hit_condition_met("garbage", 1));
    }

    #[test]
    fn log_messages_split_into_parts() {
        assert_eq!(
            parse_log_message("total={subtotal + 1}, {{literal}} {order.Customer}"),
            vec![LogPart::Text("total=".into()), LogPart::Expr("subtotal + 1".into()), LogPart::Text(", {literal} ".into()), LogPart::Expr("order.Customer".into())]
        );
        assert_eq!(parse_log_message("{new { A = 1 }.A}"), vec![LogPart::Expr("new { A = 1 }.A".into())]);
    }

    #[test]
    fn requests_responses_and_events_update_state() {
        let a = PathBuf::from("/w/A.cs");
        let mut bps = Breakpoints::new();
        bps.set(BreakpointSpec::new(&a, 10).with_condition("x > 1"));
        bps.set(BreakpointSpec::new(&a, 4).with_hit_condition(">= 2").with_log_message("x={x}"));
        bps.set(BreakpointSpec { enabled: false, ..BreakpointSpec::new(&a, 20) });
        let (lines, req) = bps.request_for(&a, Emulation { hit_conditions: true, log_points: true });
        assert_eq!(lines, vec![4, 10]);
        assert_eq!(req[0], SourceBreakpoint { line: 4, ..Default::default() });
        assert_eq!(req[1].condition.as_deref(), Some("x > 1"));
        let (_, native) = bps.request_for(&a, Emulation::default());
        assert_eq!(native[0].hit_condition.as_deref(), Some(">= 2"));
        assert_eq!(native[0].log_message.as_deref(), Some("x={x}"));

        bps.apply_response(
            &a,
            &lines,
            &[
                Breakpoint { id: Some(1), verified: false, message: Some("pending".into()), line: Some(4), ..Default::default() },
                Breakpoint { id: Some(2), verified: true, line: Some(11), ..Default::default() },
            ],
        );
        assert_eq!(bps.get(&a, 4).unwrap().message.as_deref(), Some("pending"));
        assert_eq!(bps.get(&a, 10).unwrap().display_line(), 11);
        assert_eq!(bps.apply_event(&Breakpoint { id: Some(1), verified: true, line: Some(5), ..Default::default() }), Some(a.clone()));
        let b = bps.get(&a, 4).unwrap();
        assert!(b.verified && b.message.is_none() && b.bound_line == Some(5));

        assert_eq!(bps.at_stop(&[2], None, 0), vec![(a.clone(), 10)]);
        assert_eq!(bps.at_stop(&[], Some(&a), 5), vec![(a.clone(), 4)]);
        assert!(bps.at_stop(&[], Some(&a), 20).is_empty());

        assert!(bps.remove(&a, 4));
        assert!(!bps.remove(&a, 4));
        assert_eq!(bps.in_file(&a).len(), 2);
        bps.reset_state();
        assert!(!bps.get(&a, 10).unwrap().verified);
    }
}
