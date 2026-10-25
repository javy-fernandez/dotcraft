//! Reformat Code for files the C# server does not format: JSON (re-indented, keys and
//! comments kept in place) and XML-like markup (`.csproj`, `.props`, `.config`, `.xml`), which
//! is re-indented by nesting depth. Each returns the new text or `None` when it cannot parse.

/// JSON: one value per line, `indent` per level, `": "` after keys; empty objects/arrays stay
/// `{}`/`[]`. Comments (JSONC) are kept on their own lines.
pub fn json(text: &str, indent: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut depth: usize = 0;
    let mut i = 0;
    let newline = |out: &mut String, depth: usize| {
        while out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
        for _ in 0..depth {
            out.push_str(indent);
        }
    };
    let next_significant = |mut j: usize| {
        while j < chars.len() && chars[j].is_whitespace() {
            j += 1;
        }
        chars.get(j).copied()
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                let s = i;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if i >= chars.len() {
                    return None;
                }
                out.extend(&chars[s..=i]);
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                let s = i;
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                if !out.trim_end().is_empty() && !out.ends_with('\n') && !out.trim_end().ends_with(['{', '[', ',']) {
                    out.push(' ');
                }
                out.extend(&chars[s..i]);
                newline(&mut out, depth);
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                let s = i;
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 1;
                out.extend(&chars[s..=i.min(chars.len() - 1)]);
            }
            '{' | '[' => {
                let close = if c == '{' { '}' } else { ']' };
                out.push(c);
                if next_significant(i + 1) == Some(close) {
                    // `{}` and `[]` stay together.
                    while chars[i] != close {
                        i += 1;
                    }
                    out.push(close);
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.checked_sub(1)?;
                while out.ends_with(|ch: char| ch.is_whitespace()) {
                    out.pop();
                }
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(',');
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
        i += 1;
    }
    if depth != 0 {
        return None;
    }
    let mut out = out.trim_end().to_string();
    out.push('\n');
    Some(out)
}

/// XML and MSBuild files: each line re-indented by the elements open before it. Text inside
/// comments, CDATA and multi-line attribute lists keeps its relative position.
pub fn xml(text: &str, indent: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut depth: i32 = 0;
    let mut in_comment = false;
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            out.push('\n');
            continue;
        }
        if in_comment {
            out.push_str(line);
            out.push('\n');
            if t.contains("-->") {
                in_comment = false;
            }
            continue;
        }
        // A line that starts by closing an element sits at the closed level.
        let opens_with_close = t.starts_with("</");
        let level = if opens_with_close { depth - 1 } else { depth };
        for _ in 0..level.max(0) {
            out.push_str(indent);
        }
        out.push_str(t);
        out.push('\n');
        if t.starts_with("<!--") && !t.contains("-->") {
            in_comment = true;
            continue;
        }
        depth += xml_depth_change(t);
    }
    if !text.ends_with('\n') {
        out.pop();
    }
    Some(out)
}

/// Net elements opened by a line: `<a>` +1, `</a>` −1, `<a/>`, `<?…?>`, `<!…>` and comments 0.
fn xml_depth_change(line: &str) -> i32 {
    let b = line.as_bytes();
    let mut d = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if line[i..].starts_with("<!--") {
            i = line[i..].find("-->").map_or(b.len(), |k| i + k + 3);
            continue;
        }
        let close = b.get(i + 1) == Some(&b'/');
        let special = matches!(b.get(i + 1), Some(b'?' | b'!'));
        // The tag's end, skipping quoted values.
        let mut j = i + 1;
        let mut quote = 0u8;
        while j < b.len() {
            match b[j] {
                q @ (b'"' | b'\'') if quote == 0 => quote = q,
                q if q == quote => quote = 0,
                b'>' if quote == 0 => break,
                _ => {}
            }
            j += 1;
        }
        let self_closing = j < b.len() && j > 0 && b[j - 1] == b'/';
        if !special {
            if close {
                d -= 1;
            } else if !self_closing {
                // An open tag without its `>` on this line still opens an element.
                d += 1;
            }
        }
        i = j + 1;
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_reindented_without_reordering() {
        let src = "{\"b\":1,\"a\":[1,2,{\"x\":true}],\"e\":{},\"s\":\"a,b{c}\" // note\n}";
        let out = json(src, "  ").unwrap();
        assert_eq!(out, "{\n  \"b\": 1,\n  \"a\": [\n    1,\n    2,\n    {\n      \"x\": true\n    }\n  ],\n  \"e\": {},\n  \"s\": \"a,b{c}\" // note\n}\n");
        assert!(json("{\"a\": [1, 2}", "  ").is_none());
    }

    #[test]
    fn xml_is_reindented_by_depth() {
        let src = "<Project Sdk=\"Microsoft.NET.Sdk\">\n<PropertyGroup>\n<TargetFramework>net8.0</TargetFramework>\n</PropertyGroup>\n<ItemGroup>\n    <PackageReference Include=\"A\" Version=\"1\" />\n<!-- a\ncomment -->\n</ItemGroup>\n</Project>\n";
        let out = xml(src, "  ").unwrap();
        assert_eq!(out, "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <TargetFramework>net8.0</TargetFramework>\n  </PropertyGroup>\n  <ItemGroup>\n    <PackageReference Include=\"A\" Version=\"1\" />\n    <!-- a\ncomment -->\n  </ItemGroup>\n</Project>\n");
    }
}
