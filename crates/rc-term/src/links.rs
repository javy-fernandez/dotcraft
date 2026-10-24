//! Clickable links in terminal output: `http(s)://` URLs and file paths with an optional
//! position (`path:line:col`, `path:line`, and MSBuild's `path(line,col)`).

use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Url(String),
    /// A path as printed (relative paths are relative to the terminal's directory).
    /// Line and column are 1-based.
    File { path: String, line: Option<u32>, column: Option<u32> },
}

/// Characters that never belong to a link.
fn is_boundary(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>' | '|' | '\0')
}

/// Every link of a line, with its range in chars.
pub fn find_links(line: &str) -> Vec<(Range<usize>, Link)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if is_boundary(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !is_boundary(chars[i]) {
            i += 1;
        }
        if let Some((range, link)) = token_link(&chars[start..i]) {
            out.push((start + range.start..start + range.end, link));
        }
    }
    out
}

/// The link under char index `at`, if any.
pub fn link_at(line: &str, at: usize) -> Option<(Range<usize>, Link)> {
    find_links(line).into_iter().find(|(r, _)| r.contains(&at))
}

/// Link inside a whitespace-separated token, trimmed of surrounding punctuation.
fn token_link(tok: &[char]) -> Option<(Range<usize>, Link)> {
    let mut a = 0;
    let mut b = tok.len();
    while a < b && matches!(tok[a], '(' | '[' | '{') {
        a += 1;
    }
    loop {
        if a >= b {
            return None;
        }
        let last = tok[b - 1];
        let unbalanced = |open: char, close: char| {
            let s = &tok[a..b];
            s.iter().filter(|&&c| c == close).count() > s.iter().filter(|&&c| c == open).count()
        };
        let strip = match last {
            '.' | ',' | ';' | ':' | '!' | '?' => true,
            ')' => unbalanced('(', ')'),
            ']' => unbalanced('[', ']'),
            '}' => unbalanced('{', '}'),
            _ => false,
        };
        if !strip {
            break;
        }
        b -= 1;
    }
    let text: String = tok[a..b].iter().collect();
    if text.starts_with("http://") || text.starts_with("https://") {
        if text.len() <= "https://".len() {
            return None;
        }
        return Some((a..b, Link::Url(text)));
    }
    if text.contains("://") {
        return None;
    }
    let (path, line, column) = split_position(&text);
    if !looks_like_path(path) {
        return None;
    }
    Some((a..b, Link::File { path: path.to_string(), line, column }))
}

/// `path(12,5)`, `path(12)`, `path:12:5`, `path:12` → (path, line, column).
fn split_position(text: &str) -> (&str, Option<u32>, Option<u32>) {
    let nums = |s: &str| -> Option<(u32, Option<u32>)> {
        let mut parts = s.splitn(2, [',', ':']);
        let line = parts.next()?.parse().ok()?;
        let col = match parts.next() {
            Some(c) => Some(c.parse().ok()?),
            None => None,
        };
        Some((line, col))
    };
    // MSBuild / C# compiler: File.cs(12,5)
    if let Some(inner) = text.strip_suffix(')')
        && let Some(open) = inner.rfind('(')
        && let Some((line, col)) = nums(&inner[open + 1..])
        && open > 0
    {
        return (&text[..open], Some(line), col);
    }
    // path:line[:col] — skip a Windows drive (`C:\`).
    let skip = if text.len() > 2 && text.as_bytes()[1] == b':' && text.as_bytes()[0].is_ascii_alphabetic() { 2 } else { 0 };
    if let Some(colon) = text[skip..].find(':').map(|c| c + skip)
        && colon > 0
        && let Some((line, col)) = nums(&text[colon + 1..])
    {
        return (&text[..colon], Some(line), col);
    }
    (text, None, None)
}

/// A path has a separator or a file extension starting with a letter (`Program.cs`).
fn looks_like_path(p: &str) -> bool {
    if p.is_empty() || p.contains(':') && !(p.len() > 2 && p.as_bytes()[1] == b':') {
        return false;
    }
    if !p.chars().any(|c| c.is_alphabetic()) {
        return false;
    }
    if p.contains('/') || p.contains('\\') {
        // Not just slashes or a lone `~`.
        return p.trim_matches(['/', '\\', '.', '~']).chars().any(|c| c.is_alphanumeric());
    }
    match p.rsplit_once('.') {
        Some((name, ext)) => {
            !name.is_empty()
                && (1..=10).contains(&ext.len())
                && ext.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, line: Option<u32>, column: Option<u32>) -> Link {
        Link::File { path: path.into(), line, column }
    }

    #[test]
    fn urls() {
        let l = "see https://learn.microsoft.com/dotnet/core. and (http://localhost:5000/api)";
        let links = find_links(l);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].1, Link::Url("https://learn.microsoft.com/dotnet/core".into()));
        assert_eq!(&l[links[0].0.clone()], "https://learn.microsoft.com/dotnet/core");
        assert_eq!(links[1].1, Link::Url("http://localhost:5000/api".into()));
        assert_eq!(link_at("x https://a.b/c?q=(1) y", 4).unwrap().1, Link::Url("https://a.b/c?q=(1)".into()));
        assert_eq!(link_at("x https://a.b y", 0), None);
    }

    #[test]
    fn file_positions() {
        assert_eq!(link_at("src/main.rs:12:5: warning", 3).unwrap().1, file("src/main.rs", Some(12), Some(5)));
        assert_eq!(link_at("at Program.cs:42", 5).unwrap().1, file("Program.cs", Some(42), None));
        let msbuild = "/repo/Shop/Program.cs(12,34): error CS1002: ; expected [/repo/Shop/Shop.csproj]";
        let (r, link) = link_at(msbuild, 2).unwrap();
        assert_eq!(link, file("/repo/Shop/Program.cs", Some(12), Some(34)));
        assert_eq!(&msbuild[r], "/repo/Shop/Program.cs(12,34)");
        let (_, proj) = link_at(msbuild, msbuild.len() - 5).unwrap();
        assert_eq!(proj, file("/repo/Shop/Shop.csproj", None, None));
        assert_eq!(link_at(r"C:\src\App\Foo.cs(3,1): error", 4).unwrap().1, file(r"C:\src\App\Foo.cs", Some(3), Some(1)));
        assert_eq!(link_at(r"C:\src\App\Foo.cs:7", 4).unwrap().1, file(r"C:\src\App\Foo.cs", Some(7), None));
    }

    #[test]
    fn plain_paths() {
        assert_eq!(link_at("open ./bin/Debug/net10.0/Shop.dll now", 8).unwrap().1, file("./bin/Debug/net10.0/Shop.dll", None, None));
        assert_eq!(link_at("edit appsettings.json.", 7).unwrap().1, file("appsettings.json", None, None));
        assert_eq!(link_at("'~/projects/shop'", 3).unwrap().1, file("~/projects/shop", None, None));
    }

    #[test]
    fn not_links() {
        assert!(find_links("version 1.2.3 took 0.5s ok").is_empty());
        assert!(find_links("hello world 12:30 / -- ...").is_empty());
        assert!(find_links("ftp://example.com").is_empty());
        assert!(find_links("http:// alone").is_empty());
    }
}
