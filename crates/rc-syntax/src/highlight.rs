//! Highlighting of a range of lines from a syntax tree.

use crate::lang::Lang;
use rc_text::Rope;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, QueryCursor, Tree};

/// Colour role of a piece of text. The editor theme maps each to a colour and style; LSP
/// semantic tokens map onto the same roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum Hl {
    #[default]
    Plain,
    Keyword,
    String,
    Escape,
    Number,
    Comment,
    DocComment,
    Class,
    Interface,
    Struct,
    Enum,
    Delegate,
    TypeParam,
    Namespace,
    Method,
    Field,
    Property,
    Event,
    Constant,
    EnumMember,
    Parameter,
    Local,
    Operator,
    Punct,
    Attribute,
    Preproc,
    Tag,
    XmlAttr,
    Entity,
    Heading,
    Code,
    Link,
    /// Text the language server reports as unused or inactive.
    Dimmed,
}

impl Hl {
    /// Role for a capture name of the highlight queries.
    pub fn from_capture(name: &str) -> Option<Hl> {
        Some(match name {
            "none" => Hl::Plain,
            "variable" | "variable.local" | "field.maybe" => return None,
            "keyword" => Hl::Keyword,
            "string" | "string.brace" => Hl::String,
            "string.escape" => Hl::Escape,
            "number" => Hl::Number,
            "comment" => Hl::Comment,
            "comment.doc" => Hl::DocComment,
            "type" | "type.class" => Hl::Class,
            "type.interface" => Hl::Interface,
            "type.struct" => Hl::Struct,
            "type.enum" => Hl::Enum,
            "type.delegate" => Hl::Delegate,
            "type.parameter" => Hl::TypeParam,
            "namespace" => Hl::Namespace,
            "function" | "function.call" => Hl::Method,
            "field" => Hl::Field,
            "property" => Hl::Property,
            "event" => Hl::Event,
            "constant.enum" => Hl::EnumMember,
            "variable.parameter" => Hl::Parameter,
            "operator" => Hl::Operator,
            "punctuation" | "punctuation.bracket" => Hl::Punct,
            "attribute" => Hl::Attribute,
            "xml.attribute" => Hl::XmlAttr,
            "preproc" | "preproc.arg" => Hl::Preproc,
            "tag" => Hl::Tag,
            "entity" => Hl::Entity,
            "heading" => Hl::Heading,
            "code" => Hl::Code,
            "link" => Hl::Link,
            _ => return None,
        })
    }
}

/// A coloured run within one line, in *char* columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub hl: Hl,
}

/// Highlights lines `first..first + count` of `rope`: one span list per line, covering only
/// the coloured parts (the rest is `Plain`).
pub fn highlight_lines(lang: Lang, tree: &Tree, rope: &Rope, first: usize, count: usize) -> Vec<Vec<Span>> {
    let last = (first + count).min(rope.len_lines());
    if first >= last {
        return Vec::new();
    }
    let Some(query) = lang.query() else { return vec![Vec::new(); last - first] };
    let start_byte = rope.line_to_byte(first);
    let end_byte = if last < rope.len_lines() { rope.line_to_byte(last) } else { rope.len_bytes() };
    let mut roles = vec![None::<(usize, u32, Hl)>; end_byte - start_byte];
    let names = query.capture_names();

    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(start_byte..end_byte);
    let text = |node: Node| {
        let (s, e) = (node.start_byte().min(rope.len_bytes()), node.end_byte().min(rope.len_bytes()));
        rope.byte_slice(s..e).chunks().map(str::as_bytes)
    };
    let mut captures = cursor.captures(query, tree.root_node(), text);
    // (byte len, pattern) decide precedence: smaller nodes, then later patterns win.
    while let Some((m, index)) = captures.next() {
        let cap = m.captures[*index];
        let Some(hl) = Hl::from_capture(names[cap.index as usize]) else { continue };
        let node = cap.node;
        let (s, e) = (node.start_byte().max(start_byte), node.end_byte().min(end_byte));
        if s >= e {
            continue;
        }
        let size = node.end_byte() - node.start_byte();
        let pattern = m.pattern_index as u32;
        for slot in &mut roles[s - start_byte..e - start_byte] {
            let wins = match slot {
                None => true,
                Some((sz, pat, _)) => size < *sz || (size == *sz && pattern >= *pat),
            };
            if wins {
                *slot = Some((size, pattern, hl));
            }
        }
    }

    // Razor leaves HTML to an injection: colour the markup of its elements with a lexer, where
    // the query (Razor and C#) coloured nothing.
    if lang == Lang::Razor {
        let mut elements = Vec::new();
        collect_elements(tree.root_node(), start_byte, end_byte, &mut elements);
        for (es, ee) in elements {
            // Start a little before the viewport so a tag cut by it is still recognised.
            let from = es.max(start_byte.saturating_sub(4096));
            let to = ee.min(end_byte);
            if from >= to {
                continue;
            }
            let text: Vec<u8> = rope.byte_slice(from..to).bytes().collect();
            for (s, e, hl) in html_spans(&text) {
                let (s, e) = ((from + s).max(start_byte), (from + e).min(end_byte));
                for slot in roles.get_mut(s.saturating_sub(start_byte)..e.saturating_sub(start_byte)).into_iter().flatten() {
                    // The C# patterns see a tag's `<`, `=` and `>` as operators: markup wins there.
                    if slot.is_none_or(|(_, _, h)| matches!(h, Hl::Operator | Hl::Punct)) {
                        *slot = Some((usize::MAX, 0, hl));
                    }
                }
            }
        }
    }

    // Byte roles → char spans per line.
    let mut out = Vec::with_capacity(last - first);
    for line in first..last {
        let lb = rope.line_to_byte(line) - start_byte;
        let slice = rope.line(line);
        let mut spans: Vec<Span> = Vec::new();
        let mut byte = lb;
        for (col, ch) in slice.chars().enumerate() {
            if ch == '\n' {
                break;
            }
            let hl = roles.get(byte).copied().flatten().map_or(Hl::Plain, |r| r.2);
            match spans.last_mut() {
                Some(sp) if sp.hl == hl && sp.end == col => sp.end = col + 1,
                _ => spans.push(Span { start: col, end: col + 1, hl }),
            }
            byte += ch.len_utf8();
        }
        spans.retain(|s| s.hl != Hl::Plain);
        out.push(spans);
    }
    out
}

/// Outermost HTML `element` nodes of a Razor tree that overlap `start..end` (bytes).
fn collect_elements(node: Node, start: usize, end: usize, out: &mut Vec<(usize, usize)>) {
    if node.end_byte() <= start || node.start_byte() >= end {
        return;
    }
    if node.kind() == "element" {
        out.push((node.start_byte(), node.end_byte()));
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_elements(child, start, end, out);
    }
}

/// HTML markup roles in `text` (byte ranges): tags and their punctuation, attribute names,
/// quoted values, entities. Text between tags stays plain.
pub fn html_spans(text: &[u8]) -> Vec<(usize, usize, Hl)> {
    let name_char = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b':' | b'.' | b'@');
    let mut out = Vec::new();
    let mut i = 0;
    let n = text.len();
    while i < n {
        match text[i] {
            b'<' if i + 1 < n && (text[i + 1].is_ascii_alphabetic() || text[i + 1] == b'/') => {
                let start = i;
                i += 1;
                if text.get(i) == Some(&b'/') {
                    i += 1;
                }
                while i < n && name_char(text[i]) {
                    i += 1;
                }
                out.push((start, i, Hl::Tag));
                // Attributes up to the closing `>` or `/>`.
                while i < n {
                    let c = text[i];
                    if c.is_ascii_whitespace() {
                        i += 1;
                    } else if c == b'>' {
                        out.push((i, i + 1, Hl::Tag));
                        i += 1;
                        break;
                    } else if c == b'/' && text.get(i + 1) == Some(&b'>') {
                        out.push((i, i + 2, Hl::Tag));
                        i += 2;
                        break;
                    } else if c == b'=' {
                        out.push((i, i + 1, Hl::Tag));
                        i += 1;
                    } else if c == b'"' || c == b'\'' {
                        let s = i;
                        i += 1;
                        while i < n && text[i] != c {
                            i += 1;
                        }
                        i = (i + 1).min(n);
                        out.push((s, i, Hl::String));
                    } else if name_char(c) {
                        let s = i;
                        while i < n && name_char(text[i]) {
                            i += 1;
                        }
                        out.push((s, i, Hl::XmlAttr));
                    } else if c == b'<' {
                        break;
                    } else {
                        i += 1;
                    }
                }
            }
            b'&' => {
                let s = i;
                let mut j = i + 1;
                while j < n && j - s < 12 && (text[j].is_ascii_alphanumeric() || text[j] == b'#') {
                    j += 1;
                }
                if j < n && text[j] == b';' && j > s + 1 {
                    out.push((s, j + 1, Hl::Entity));
                    i = j + 1;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// Highlighting of languages without a grammar (`.sln`); `None` for the others.
pub fn highlight_lexical(lang: Lang, rope: &Rope, first: usize, count: usize) -> Option<Vec<Vec<Span>>> {
    if lang != Lang::Sln {
        return None;
    }
    let last = (first + count).min(rope.len_lines());
    Some((first..last).map(|l| sln_line(&rope.line(l).to_string())).collect())
}

/// One line of a `.sln`: comments, section keywords, strings, GUIDs and version numbers.
fn sln_line(line: &str) -> Vec<Span> {
    let chars: Vec<char> = line.trim_end_matches(['\n', '\r']).chars().collect();
    let mut roles = vec![Hl::Plain; chars.len()];
    let lead = chars.iter().take_while(|c| c.is_whitespace()).count();
    if chars.get(lead) == Some(&'#') {
        roles[lead..].fill(Hl::Comment);
    } else {
        const KEYWORDS: &[&str] = &["Project", "EndProject", "ProjectSection", "EndProjectSection", "Global", "EndGlobal", "GlobalSection", "EndGlobalSection", "preSolution", "postSolution", "preProject", "postProject"];
        const PROPERTIES: &[&str] = &["VisualStudioVersion", "MinimumVisualStudioVersion"];
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '"' {
                let s = i;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    i += 1;
                }
                i = (i + 1).min(chars.len());
                roles[s..i].fill(Hl::String);
            } else if c.is_ascii_alphabetic() {
                let s = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[s..i].iter().collect();
                if KEYWORDS.contains(&word.as_str()) {
                    roles[s..i].fill(Hl::Keyword);
                } else if PROPERTIES.contains(&word.as_str()) {
                    roles[s..i].fill(Hl::Property);
                }
            } else if c.is_ascii_digit() {
                let s = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                roles[s..i].fill(Hl::Number);
            } else if c == '=' {
                roles[i] = Hl::Operator;
                i += 1;
            } else {
                i += 1;
            }
        }
        // GUIDs, also inside strings: `{8BC9CEB8-8B4A-11D0-8D11-00A0C91BC942}`.
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '{'
                && let Some(len) = chars[i..].iter().position(|&c| c == '}')
                && len == 37
                && chars[i + 1..i + len].iter().all(|c| c.is_ascii_hexdigit() || *c == '-')
            {
                roles[i..=i + len].fill(Hl::Constant);
                i += len + 1;
            } else {
                i += 1;
            }
        }
    }
    let mut spans: Vec<Span> = Vec::new();
    for (col, hl) in roles.into_iter().enumerate() {
        match spans.last_mut() {
            Some(sp) if sp.hl == hl && sp.end == col => sp.end = col + 1,
            _ => spans.push(Span { start: col, end: col + 1, hl }),
        }
    }
    spans.retain(|s| s.hl != Hl::Plain);
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_now;

    fn roles_of(lang: Lang, src: &str) -> Vec<(String, Hl)> {
        let rope = Rope::from_str(src);
        let tree = parse_now(lang, &rope, None).unwrap();
        let lines = highlight_lines(lang, &tree, &rope, 0, rope.len_lines());
        let mut out = Vec::new();
        for (i, spans) in lines.iter().enumerate() {
            let line: Vec<char> = rope.line(i).chars().collect();
            for s in spans {
                out.push((line[s.start..s.end].iter().collect(), s.hl));
            }
        }
        out
    }

    #[test]
    fn csharp_roles_follow_rider() {
        let src = "using System.Text;\nnamespace Demo;\n/// <summary>Doc</summary>\npublic sealed class Greeter : IGreeter\n{\n    private readonly int _count = 42;\n    public string Name { get; set; }\n    public void Say(string who) { Console.WriteLine($\"Hi {who}\\n\"); }\n}\n";
        let roles = roles_of(Lang::CSharp, src);
        let has = |text: &str, hl: Hl| roles.iter().any(|(t, h)| t == text && *h == hl);
        assert!(has("using", Hl::Keyword), "{roles:?}");
        assert!(has("System", Hl::Namespace) && has("Text", Hl::Namespace), "{roles:?}");
        assert!(has("public sealed", Hl::Keyword) || has("public", Hl::Keyword), "{roles:?}");
        assert!(has("Greeter", Hl::Class), "{roles:?}");
        assert!(has("IGreeter", Hl::Class), "{roles:?}");
        assert!(has("_count", Hl::Field), "{roles:?}");
        assert!(has("42", Hl::Number), "{roles:?}");
        assert!(has("Name", Hl::Property), "{roles:?}");
        assert!(has("Say", Hl::Method), "{roles:?}");
        assert!(has("WriteLine", Hl::Method), "{roles:?}");
        assert!(has("who", Hl::Parameter), "{roles:?}");
        assert!(has("\\n", Hl::Escape), "{roles:?}");
        assert!(roles.iter().any(|(t, h)| t.starts_with("///") && *h == Hl::DocComment), "{roles:?}");
    }

    #[test]
    fn xml_roles() {
        let roles = roles_of(Lang::Xml, "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <!-- c -->\n</Project>\n");
        assert!(roles.iter().any(|(t, h)| t == "Sdk" && *h == Hl::XmlAttr), "{roles:?}");
        assert!(roles.iter().any(|(t, h)| t == "\"Microsoft.NET.Sdk\"" && *h == Hl::String), "{roles:?}");
        assert!(roles.iter().any(|(t, h)| t == "<!-- c -->" && *h == Hl::Comment), "{roles:?}");
    }

    #[test]
    fn razor_colours_directives_csharp_and_html() {
        let src = "@page \"/orders\"\n@inject OrderService Orders\n\n<h1 class=\"title\">Orders &amp; more</h1>\n@if (count > 0)\n{\n    <p>@count items</p>\n}\n\n@code {\n    private int count = 3;\n}\n";
        let roles = roles_of(Lang::Razor, src);
        let has = |text: &str, hl: Hl| roles.iter().any(|(t, h)| t == text && *h == hl);
        assert!(roles.iter().any(|(t, h)| t.starts_with("@page") && *h == Hl::Keyword) || has("@page", Hl::Keyword), "{roles:?}");
        assert!(has("\"/orders\"", Hl::String), "{roles:?}");
        assert!(has("<h1", Hl::Tag) && has("</h1>", Hl::Tag) && has("<p>", Hl::Tag), "{roles:?}");
        assert!(has("class", Hl::XmlAttr), "{roles:?}");
        assert!(has("\"title\"", Hl::String), "{roles:?}");
        assert!(has("&amp;", Hl::Entity), "{roles:?}");
        assert!(roles.iter().any(|(t, h)| t.contains("@if") && *h == Hl::Keyword), "{roles:?}");
        assert!(has("int", Hl::Keyword) && has("3", Hl::Number), "{roles:?}");
    }

    #[test]
    fn solution_files_get_keywords_strings_and_guids() {
        let src = "Microsoft Visual Studio Solution File, Format Version 12.00\n# Visual Studio Version 17\nVisualStudioVersion = 17.0.31903.59\nProject(\"{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}\") = \"App\", \"App\\App.csproj\", \"{0A1B2C3D-0000-0000-0000-000000000001}\"\nEndProject\nGlobal\n\tGlobalSection(SolutionConfigurationPlatforms) = preSolution\n\tEndGlobalSection\nEndGlobal\n";
        let rope = Rope::from_str(src);
        let lines = highlight_lexical(Lang::Sln, &rope, 0, rope.len_lines()).unwrap();
        let text = |l: usize, s: &Span| rope.line(l).chars().skip(s.start).take(s.end - s.start).collect::<String>();
        let found = |l: usize, want: &str, hl: Hl| lines[l].iter().any(|s| s.hl == hl && text(l, s) == want);
        assert!(found(1, "# Visual Studio Version 17", Hl::Comment));
        assert!(found(2, "VisualStudioVersion", Hl::Property) && found(2, "17.0.31903.59", Hl::Number));
        assert!(found(3, "Project", Hl::Keyword) && found(3, "{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}", Hl::Constant) && found(3, "\"App\"", Hl::String));
        assert!(found(6, "GlobalSection", Hl::Keyword) && found(6, "preSolution", Hl::Keyword));
        assert!(highlight_lexical(Lang::CSharp, &rope, 0, 1).is_none());
    }
}
