//! Parser for MSBuild canonical messages (compiler, MSBuild, NuGet and SDK errors):
//! `origin: [subcategory] category code: text [project]`.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// One build diagnostic. Lines and columns are 1-based, as MSBuild prints them.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    /// Source file, when the origin is a path (`None` for `MSBUILD`, `CSC`…).
    pub file: Option<PathBuf>,
    /// The origin as written (`/p/File.cs`, `MSBUILD`, `CSC`…).
    pub origin: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub end_line: Option<u32>,
    pub end_column: Option<u32>,
    pub severity: Severity,
    /// `CS0103`, `NU1101`, `MSB1009`… (empty when the message has none).
    pub code: String,
    pub message: String,
    /// Project in the trailing `[...]`, when present.
    pub project: Option<PathBuf>,
}

/// Parses one output line; `None` when it is not a diagnostic. ANSI sequences are ignored.
pub fn parse_line(line: &str) -> Option<Diagnostic> {
    let clean = crate::process::strip_ansi(line);
    let line = clean.trim();
    if line.is_empty() {
        return None;
    }
    // Messages without origin: "error MSB1009: ...".
    // (Requires a code, so that app logs such as "info: Host[0]" are not taken for one.)
    if let Some((sev, code, rest)) = parse_category(line)
        && sev != Severity::Info
        && !code.is_empty()
    {
        return Some(finish(String::new(), sev, code, rest));
    }
    let mut search = 0;
    while let Some(off) = line[search..].find(':') {
        let colon = search + off;
        search = colon + 1;
        let origin = line[..colon].trim_end();
        if origin.is_empty() {
            continue;
        }
        if let Some((sev, code, rest)) = parse_category(line[colon + 1..].trim_start()) {
            return Some(finish(origin.to_string(), sev, code, rest));
        }
    }
    None
}

/// Parses every diagnostic in `text`, dropping exact duplicates (MSBuild repeats them).
pub fn parse_all(text: &str) -> Vec<Diagnostic> {
    let mut out: Vec<Diagnostic> = Vec::new();
    for d in text.lines().filter_map(parse_line) {
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

/// `[subcategory ]category[ code]: rest` → (severity, code, rest).
fn parse_category(s: &str) -> Option<(Severity, String, &str)> {
    let colon = s.find(':')?;
    let head = &s[..colon];
    if head.len() > 60 {
        return None;
    }
    let words: Vec<&str> = head.split_whitespace().collect();
    // Find the category word; anything before it is a subcategory, at most one word after it (the code).
    let idx = words.iter().position(|w| matches!(*w, "error" | "warning" | "info" | "message"))?;
    if idx > 2 || words.len() > idx + 2 {
        return None;
    }
    let severity = match words[idx] {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        _ => Severity::Info,
    };
    let code = words.get(idx + 1).copied().unwrap_or("");
    if !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-') {
        return None;
    }
    // Subcategory words are plain words ("fatal"), never paths.
    if words[..idx].iter().any(|w| !w.chars().all(|c| c.is_alphabetic())) {
        return None;
    }
    Some((severity, code.to_string(), s[colon + 1..].trim()))
}

fn finish(origin: String, severity: Severity, code: String, rest: &str) -> Diagnostic {
    let (message, project) = split_project(rest);
    let mut d = Diagnostic {
        file: None,
        origin: origin.clone(),
        line: None,
        column: None,
        end_line: None,
        end_column: None,
        severity,
        code,
        message: message.to_string(),
        project,
    };
    let (path, pos) = split_position(&origin);
    if let Some([l, c, el, ec]) = pos {
        d.line = l;
        d.column = c;
        d.end_line = el;
        d.end_column = ec;
    }
    if looks_like_path(path) {
        d.file = Some(PathBuf::from(path));
    }
    d
}

/// `msg [/p/A.csproj]` → (`msg`, Some(project)).
fn split_project(rest: &str) -> (&str, Option<PathBuf>) {
    if let Some(stripped) = rest.strip_suffix(']')
        && let Some(open) = stripped.rfind(" [")
    {
        let inner = &stripped[open + 2..];
        if looks_like_path(inner) {
            return (stripped[..open].trim_end(), Some(PathBuf::from(inner)));
        }
    }
    (rest, None)
}

type Position = [Option<u32>; 4];

/// `File.cs(12,5)` → (`File.cs`, [12, 5, -, -]); also `(l)`, `(l-el)`, `(l,c-ec)`, `(l,c,el,ec)`.
fn split_position(origin: &str) -> (&str, Option<Position>) {
    let Some(inner) = origin.strip_suffix(')') else { return (origin, None) };
    let Some(open) = inner.rfind('(') else { return (origin, None) };
    let nums = &inner[open + 1..];
    let parse = |s: &str| s.trim().parse::<u32>().ok();
    let parts: Vec<&str> = nums.split(',').collect();
    let pos = match parts.as_slice() {
        [l] => match l.split_once('-') {
            Some((a, b)) => [parse(a), None, parse(b), None],
            None => [parse(l), None, None, None],
        },
        [l, c] => match c.split_once('-') {
            Some((a, b)) => [parse(l), parse(a), parse(l), parse(b)],
            None => [parse(l), parse(c), None, None],
        },
        [l, c, el, ec] => [parse(l), parse(c), parse(el), parse(ec)],
        _ => return (origin, None),
    };
    if pos[0].is_none() {
        return (origin, None);
    }
    (inner[..open].trim_end(), Some(pos))
}

fn looks_like_path(s: &str) -> bool {
    !s.is_empty() && (s.contains('/') || s.contains('\\') || s.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compiler_errors_with_position_and_project() {
        let d = parse_line("/w/Shop/src/Shop.Core/Order.cs(12,5): error CS0103: The name 'x' does not exist in the current context [/w/Shop/src/Shop.Core/Shop.Core.csproj]").unwrap();
        assert_eq!(d.file, Some(PathBuf::from("/w/Shop/src/Shop.Core/Order.cs")));
        assert_eq!((d.line, d.column, d.end_line, d.end_column), (Some(12), Some(5), None, None));
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.code, "CS0103");
        assert_eq!(d.message, "The name 'x' does not exist in the current context");
        assert_eq!(d.project, Some(PathBuf::from("/w/Shop/src/Shop.Core/Shop.Core.csproj")));
    }

    #[test]
    fn parses_warnings_ranges_and_indented_lines() {
        let d = parse_line("    /p/A.cs(3,10,3,14): warning CS0168: The variable 'e' is declared but never used [/p/A.csproj]").unwrap();
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!((d.line, d.column, d.end_line, d.end_column), (Some(3), Some(10), Some(3), Some(14)));
        let d = parse_line("/p/B.cs(7): warning CA1000: x").unwrap();
        assert_eq!((d.line, d.column), (Some(7), None));
        assert_eq!(d.project, None);
        let d = parse_line("/p/C.cs(7-9): error X1: y").unwrap();
        assert_eq!((d.line, d.end_line), (Some(7), Some(9)));
    }

    #[test]
    fn parses_tool_origins_and_missing_codes() {
        let d = parse_line("MSBUILD : error MSB1009: Project file does not exist.").unwrap();
        assert_eq!(d.origin, "MSBUILD");
        assert_eq!(d.file, None);
        assert_eq!(d.code, "MSB1009");
        assert_eq!(d.message, "Project file does not exist.");
        let d = parse_line("CSC : error CS2001: Source file '/p/X.cs' could not be found. [/p/P.csproj]").unwrap();
        assert_eq!((d.origin.as_str(), d.file.as_ref()), ("CSC", None));
        let d = parse_line("EXEC : fatal error : something broke").unwrap();
        assert_eq!((d.severity, d.code.as_str(), d.message.as_str()), (Severity::Error, "", "something broke"));
        let d = parse_line("error MSB4025: The project file could not be loaded.").unwrap();
        assert_eq!((d.origin.as_str(), d.code.as_str()), ("", "MSB4025"));
    }

    #[test]
    fn parses_nuget_and_sdk_errors_without_position() {
        let d = parse_line("/w/App/App.csproj : error NU1101: Unable to find package Nope. No packages exist with this id in source(s): nuget.org [/w/App.sln]").unwrap();
        assert_eq!(d.file, Some(PathBuf::from("/w/App/App.csproj")));
        assert_eq!(d.line, None);
        assert_eq!(d.code, "NU1101");
        assert_eq!(d.project, Some(PathBuf::from("/w/App.sln")));
        let d = parse_line("/usr/local/share/dotnet/sdk/8.0.418/Sdks/Microsoft.NET.Sdk/targets/Microsoft.NET.TargetFrameworkInference.targets(166,5): error NETSDK1045: The current .NET SDK does not support targeting .NET 11.0. [/p/P.csproj]").unwrap();
        assert_eq!((d.code.as_str(), d.line, d.column), ("NETSDK1045", Some(166), Some(5)));
        let d = parse_line("/p/P.csproj : warning NU1603: P depends on X (>= 1.0) but X 1.0 was not found.").unwrap();
        assert_eq!(d.severity, Severity::Warning);
        // The message itself contains ": " and parentheses.
        assert_eq!(d.message, "P depends on X (>= 1.0) but X 1.0 was not found.");
    }

    #[test]
    fn parses_windows_paths() {
        let d = parse_line(r"C:\src\App\Program.cs(3,1): error CS1002: ; expected [C:\src\App\App.csproj]").unwrap();
        assert_eq!(d.file, Some(PathBuf::from(r"C:\src\App\Program.cs")));
        assert_eq!((d.line, d.column), (Some(3), Some(1)));
        assert_eq!(d.message, "; expected");
        assert_eq!(d.project, Some(PathBuf::from(r"C:\src\App\App.csproj")));
    }

    #[test]
    fn ignores_ordinary_output_and_dedupes() {
        for l in [
            "  Shop.Core -> /w/bin/Debug/net8.0/Shop.Core.dll",
            "Build succeeded.",
            "    0 Error(s)",
            "  Determining projects to restore...",
            "Time Elapsed 00:00:02.64",
            "info: Microsoft.Hosting.Lifetime[14]",
            "",
        ] {
            assert_eq!(parse_line(l), None, "{l}");
        }
        let line = "/p/A.cs(1,1): error CS1: x [/p/A.csproj]";
        assert_eq!(parse_all(&format!("{line}\nother\n{line}\n")).len(), 1);
        let d = parse_line("\x1b[31m/p/A.cs(1,2): error CS1: x\x1b[0m").unwrap();
        assert_eq!(d.column, Some(2));
    }
}
