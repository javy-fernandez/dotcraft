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
}
