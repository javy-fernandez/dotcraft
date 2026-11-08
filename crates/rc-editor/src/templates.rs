//! C# live templates (Rider's names): typing the abbreviation and Tab expands it, ⌘J lists
//! them, and ⌥⌘T surrounds the selected lines with one. Bodies use snippet syntax, `\t` for one
//! indent level and `$SELECTION$` for the surrounded lines.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Template {
    pub key: &'static str,
    pub description: &'static str,
    pub body: &'static str,
}

impl Template {
    pub fn surrounds(&self) -> bool {
        self.body.contains("$SELECTION$")
    }
}

const fn t(key: &'static str, description: &'static str, body: &'static str) -> Template {
    Template { key, description, body }
}

/// Live templates, expanded with Tab.
pub const CSHARP: &[Template] = &[
    t("prop", "Auto-property", "public ${1:int} ${2:Name} { get; set; }$0"),
    t("propg", "Auto-property with a private setter", "public ${1:int} ${2:Name} { get; private set; }$0"),
    t("propi", "Init-only auto-property", "public ${1:int} ${2:Name} { get; init; }$0"),
    t("ctor", "Constructor", "public $CLASS$($1)\n{\n\t$0\n}"),
    t("cw", "Console.WriteLine", "Console.WriteLine($0);"),
    t("for", "Simple for loop", "for (var ${1:i} = 0; $1 < ${2:length}; $1++)\n{\n\t$0\n}"),
    t("forr", "Reverse for loop", "for (var ${1:i} = ${2:length} - 1; $1 >= 0; $1--)\n{\n\t$0\n}"),
    t("foreach", "foreach loop", "foreach (var ${1:item} in ${2:collection})\n{\n\t$0\n}"),
    t("if", "if statement", "if (${1:condition})\n{\n\t$0\n}"),
    t("else", "else statement", "else\n{\n\t$0\n}"),
    t("while", "while loop", "while (${1:condition})\n{\n\t$0\n}"),
    t("do", "do…while loop", "do\n{\n\t$0\n} while (${1:condition});"),
    t("try", "try/catch", "try\n{\n\t$0\n}\ncatch (${1:Exception} ${2:e})\n{\n\tthrow;\n}"),
    t("tryf", "try/finally", "try\n{\n\t$0\n}\nfinally\n{\n\t$1\n}"),
    t("using", "using statement", "using (${1:resource})\n{\n\t$0\n}"),
    t("lock", "lock statement", "lock (${1:_sync})\n{\n\t$0\n}"),
    t("switch", "switch statement", "switch (${1:value})\n{\n\tcase ${2:0}:\n\t\t$0\n\t\tbreak;\n\tdefault:\n\t\tbreak;\n}"),
    t("class", "Class", "public class ${1:$FILE$}\n{\n\t$0\n}"),
    t("interface", "Interface", "public interface ${1:I$FILE$}\n{\n\t$0\n}"),
    t("struct", "Struct", "public struct ${1:$FILE$}\n{\n\t$0\n}"),
    t("record", "Record", "public record ${1:$FILE$}(${2:string Name});$0"),
    t("enum", "Enum", "public enum ${1:$FILE$}\n{\n\t$0\n}"),
    t("exception", "Exception class", "public class ${1:My}Exception : Exception\n{\n\tpublic $1Exception() { }\n\tpublic $1Exception(string message) : base(message) { }\n\tpublic $1Exception(string message, Exception inner) : base(message, inner) { }\n}$0"),
    t("attribute", "Attribute class", "[AttributeUsage(AttributeTargets.${1:All})]\npublic sealed class ${2:My}Attribute : Attribute\n{\n\t$0\n}"),
    t("svm", "static void Main", "static void Main(string[] args)\n{\n\t$0\n}"),
    t("sim", "static int Main", "static int Main(string[] args)\n{\n\t$0\n\treturn 0;\n}"),
    t("asrt", "Debug.Assert", "System.Diagnostics.Debug.Assert(${1:condition});$0"),
    t("nguid", "New GUID", "$GUID$"),
    t("ns", "File-scoped namespace", "namespace ${1:Name};\n$0"),
    t("fact", "xUnit test", "[Fact]\npublic void ${1:Test}()\n{\n\t$0\n}"),
    t("test", "NUnit test", "[Test]\npublic void ${1:Test}()\n{\n\t$0\n}"),
    t("testm", "MSTest test", "[TestMethod]\npublic void ${1:Test}()\n{\n\t$0\n}"),
];

/// Surround With (⌥⌘T).
pub const SURROUND: &[Template] = &[
    t("if", "if", "if (${1:condition})\n{\n\t$SELECTION$\n}$0"),
    t("ifelse", "if/else", "if (${1:condition})\n{\n\t$SELECTION$\n}\nelse\n{\n\t$0\n}"),
    t("while", "while", "while (${1:condition})\n{\n\t$SELECTION$\n}$0"),
    t("foreach", "foreach", "foreach (var ${1:item} in ${2:collection})\n{\n\t$SELECTION$\n}$0"),
    t("for", "for", "for (var ${1:i} = 0; $1 < ${2:length}; $1++)\n{\n\t$SELECTION$\n}$0"),
    t("try", "try/catch", "try\n{\n\t$SELECTION$\n}\ncatch (${1:Exception} ${2:e})\n{\n\t$0throw;\n}"),
    t("tryf", "try/finally", "try\n{\n\t$SELECTION$\n}\nfinally\n{\n\t$0\n}"),
    t("using", "using", "using (${1:resource})\n{\n\t$SELECTION$\n}$0"),
    t("lock", "lock", "lock (${1:_sync})\n{\n\t$SELECTION$\n}$0"),
    t("region", "#region", "#region ${1:Name}\n$SELECTION$\n#endregion$0"),
    t("{}", "Braces", "{\n\t$SELECTION$\n}$0"),
];

pub fn find(key: &str) -> Option<&'static Template> {
    CSHARP.iter().find(|t| t.key == key)
}

/// Escapes text for a snippet body.
pub fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('$', "\\$").replace('}', "\\}")
}

/// Fills the template's own variables: `$CLASS$`/`$FILE$` (the file name) and `$GUID$`.
pub fn fill(body: &str, file_stem: &str) -> String {
    let mut s = body.replace("$CLASS$", file_stem).replace("$FILE$", file_stem);
    if s.contains("$GUID$") {
        s = s.replace("$GUID$", &new_guid());
    }
    s
}

/// A random version-4 GUID (time and address entropy are enough for a template).
fn new_guid() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

/// Surround With: the selected lines (or the caret's line) go inside `t`, one level deeper.
pub fn surround(state: &mut crate::EditorState, t: &Template) {
    let rope = state.buffer.rope().clone();
    let r = state.selection.primary();
    let first = rope.char_to_line(r.start());
    let mut last = rope.char_to_line(r.end());
    if last > first && r.end() == rope.line_to_char(last) {
        last -= 1;
    }
    let base = rc_text::commands::line_indent(&rope, first);
    let start = rope.line_to_char(first) + base.chars().count();
    let line_end = |l: usize| {
        let s = rope.line(l).to_string();
        rope.line_to_char(l) + s.trim_end_matches(['\n', '\r']).chars().count()
    };
    let end = line_end(last).max(start);
    let lines: Vec<String> = (first..=last)
        .map(|l| {
            let s = rope.line(l).to_string();
            let s = s.trim_end_matches(['\n', '\r']);
            s.strip_prefix(base.as_str()).unwrap_or(s.trim_start()).to_string()
        })
        .collect();
    let inner = escape(&lines.join("\n")).replace('\n', "\n\t");
    let stem = state.path.as_ref().and_then(|p| p.file_stem()).map_or_else(|| "Name".to_string(), |s| s.to_string_lossy().into_owned());
    let body = fill(&t.body.replace("$SELECTION$", &inner), &stem);
    state.insert_snippet(start, end, &body);
}

/// ⌘J: inserts `t` at the caret.
pub fn insert(state: &mut crate::EditorState, t: &Template) {
    let caret = state.caret();
    let stem = state.path.as_ref().and_then(|p| p.file_stem()).map_or_else(|| "Name".to_string(), |s| s.to_string_lossy().into_owned());
    state.insert_snippet(caret, caret, &fill(t.body, &stem));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_parses_and_keys_are_unique() {
        for list in [CSHARP, SURROUND] {
            let mut keys: Vec<&str> = list.iter().map(|t| t.key).collect();
            keys.sort();
            keys.dedup();
            assert_eq!(keys.len(), list.len());
            for t in list {
                let p = crate::snippet::parse(&fill(&t.body.replace("$SELECTION$", "x"), "Order"));
                assert!(!p.text.contains('$'), "{}: {}", t.key, p.text);
            }
        }
        assert_eq!(new_guid().len(), 36);
    }
}

#[cfg(test)]
mod editor_tests {
    use crate::state::EditorState;
    use crate::{Action, EditorConfig};
    use rc_text::Selection;

    fn run(state: &mut EditorState, a: Action) {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            crate::run_action(ui, state, &EditorConfig::default(), a);
        });
        out.textures_delta.clear();
    }

    fn type_str(state: &mut EditorState, s: &str) {
        for ch in s.chars() {
            let rope = state.buffer.rope().clone();
            let e = rc_text::commands::type_char(&rope, &state.selection, ch, false);
            state.apply(e, rc_text::EditKind::Insert);
        }
    }

    #[test]
    fn workspace_templates_expand_with_tab_and_support_undo() {
        let mut state = EditorState::new(7, Some("Sample.cs".into()), "greet");
        std::sync::Arc::make_mut(&mut state.custom_templates).insert("greet".into(), "Console.WriteLine(${1:name});$0".into());
        state.selection = Selection::point(5);
        run(&mut state, Action::Indent);
        assert_eq!(state.buffer.rope().to_string(), "Console.WriteLine(name);");
        assert_eq!(state.selected_text(), "name");
        state.undo();
        assert_eq!(state.buffer.rope().to_string(), "greet");
    }

    #[test]
    fn tab_expands_moves_through_fields_and_mirrors() {
        let mut s = EditorState::new(1, Some("/tmp/Order.cs".into()), "class Order\n{\n    void M()\n    {\n        for\n    }\n}\n");
        let at = s.buffer.rope().to_string().find("for\n").unwrap() + 3;
        s.selection = Selection::point(at);
        run(&mut s, Action::Indent);
        let text = s.buffer.rope().to_string();
        assert!(text.contains("        for (var i = 0; i < length; i++)\n        {\n            \n        }"), "{text}");
        // The first field (i, three times) is selected; typing renames all of them.
        assert_eq!(s.selection.len(), 3);
        type_str(&mut s, "k");
        assert!(s.buffer.rope().to_string().contains("for (var k = 0; k < length; k++)"), "{}", s.buffer.rope());
        // Tab: the length, then the body.
        run(&mut s, Action::Indent);
        type_str(&mut s, "n");
        run(&mut s, Action::Indent);
        assert!(s.snippet.is_none());
        type_str(&mut s, "x();");
        assert!(s.buffer.rope().to_string().contains("for (var k = 0; k < n; k++)\n        {\n            x();\n        }"), "{}", s.buffer.rope());
        // Surround With: two statements go inside an if, one level deeper.
        let mut s2 = EditorState::new(3, Some("/tmp/A.cs".into()), "void M()\n{\n    a();\n    b();\n}\n");
        s2.selection = Selection::single(rc_text::Range::new(13, 29));
        super::surround(&mut s2, super::SURROUND.iter().find(|t| t.key == "if").unwrap());
        assert_eq!(s2.buffer.rope().to_string(), "void M()\n{\n    if (condition)\n    {\n        a();\n        b();\n    }\n}\n");
        assert_eq!(s2.buffer.rope().slice(s2.selection.primary().start()..s2.selection.primary().end()).to_string(), "condition");
        // Not after a dot.
        let mut s = EditorState::new(2, Some("/tmp/A.cs".into()), "a.for");
        s.selection = Selection::point(5);
        run(&mut s, Action::Indent);
        assert!(!s.buffer.rope().to_string().contains("var i"));
    }
}

/// Explicit C# postfix transformations for identifier/member expressions on their own line.
/// No type inference is attempted; calls, literals, comments, and multi-caret edits are excluded.
pub fn expand_postfix(state: &mut crate::EditorState) -> bool {
    if state.read_only || state.lang() != rc_syntax::Lang::CSharp || state.selection.len() != 1 || !state.selection.primary().is_empty() { return false; }
    // An edited but not yet reparsed tree cannot prove we are outside a multiline string/comment.
    if !state.syntax.is_current() { return false; }
    let rope = state.buffer.rope();
    let caret = state.caret();
    let byte = rope.char_to_byte(caret.saturating_sub(1));
    let mut node = state.syntax.tree().and_then(|tree| tree.root_node().descendant_for_byte_range(byte, byte));
    while let Some(current) = node {
        if current.kind().contains("comment") || current.kind().contains("string") || current.kind() == "character_literal" { return false; }
        node = current.parent();
    }
    let line = rope.char_to_line(caret);
    let start = rope.line_to_char(line);
    if caret - start > 4096 { return false; }
    let prefix = rope.slice(start..caret).to_string();
    let suffix = rope.slice(caret..(start + rope.line(line).len_chars())).to_string();
    if !suffix.trim().is_empty() { return false; }
    let trimmed = prefix.trim_start_matches([' ', '\t']);
    let Some((expression, key)) = trimmed.rsplit_once('.') else { return false };
    if !expression.split('.').all(|part| {
        let mut chars = part.chars();
        chars.next().is_some_and(|c| c == '_' || c.is_alphabetic()) && chars.all(|c| c == '_' || c.is_alphanumeric())
    }) { return false; }
    let value = escape(expression);
    let body = match key {
        "if" => format!("if ({value})\n{{\n\t$0\n}}"),
        "not" => format!("!({value})$0"),
        "null" => format!("if ({value} is null)\n{{\n\t$0\n}}"),
        "notnull" => format!("if ({value} is not null)\n{{\n\t$0\n}}"),
        "return" => format!("return {value};$0"),
        "await" => format!("await {value};$0"),
        _ => return false,
    };
    let indent = prefix.chars().count() - trimmed.chars().count();
    state.insert_snippet(start + indent, caret, &body);
    true
}

#[cfg(test)]
mod postfix_tests {
    use super::*;
    #[test]
    fn postfix_expands_unicode_members_and_undoes_without_touching_other_lines() {
        let source = "void M()\n{\n    usuário.Ready.if\n}\n";
        let mut state = crate::EditorState::new(5, Some("App.cs".into()), source);
        let caret = source[..source.find(".if").unwrap()+3].chars().count();
        state.selection = rc_text::Selection::point(caret);
        state.parse_blocking();
        assert!(expand_postfix(&mut state));
        assert!(state.buffer.rope().to_string().contains("if (usuário.Ready)\n    {"));
        state.undo();
        assert_eq!(state.buffer.rope().to_string(), source);
        for source in ["// value.if", "\"value.if", "Call().if", "left + right.if", "value.if;", "a.for"] {
            let mut state = crate::EditorState::new(6, Some("App.cs".into()), source);
            state.selection = rc_text::Selection::point(source.chars().count());
            state.parse_blocking();
            assert!(!expand_postfix(&mut state), "{source}");
            assert_eq!(state.buffer.rope().to_string(), source);
        }
    }
}

#[cfg(test)]
mod postfix_context_tests {
    #[test]
    fn postfix_requires_current_code_and_handles_all_supported_expansions() {
        for (key, expected) in [("if", "if (value)"), ("not", "!(value)"), ("null", "if (value is null)"), ("notnull", "if (value is not null)"), ("return", "return value;"), ("await", "await value;")] {
            let source = format!("value.{key}");
            let mut state = crate::EditorState::new(9, Some("App.cs".into()), &source);
            state.selection = rc_text::Selection::point(source.chars().count());
            assert!(!super::expand_postfix(&mut state));
            state.parse_blocking();
            assert!(super::expand_postfix(&mut state), "{key}");
            assert!(state.buffer.rope().to_string().starts_with(expected), "{key}");
        }
        for source in ["/*\nvalue.if\n*/", "var text = @\"\nvalue.if\n\";", "var text = \"\"\"\nvalue.if\n\"\"\";"] {
            let mut state = crate::EditorState::new(10, Some("App.cs".into()), source);
            state.selection = rc_text::Selection::point(source[..source.find(".if").unwrap()+3].chars().count());
            state.parse_blocking();
            assert!(!super::expand_postfix(&mut state), "{source}");
            assert_eq!(state.buffer.rope().to_string(), source);
        }
    }
}
