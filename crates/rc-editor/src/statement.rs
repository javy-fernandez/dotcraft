//! Conservative C# statement completion for the editor shortcut.

use crate::EditorState;
use rc_text::commands::{self, Edited};
use rc_text::{Change, ChangeSet, EditKind, Selection};

/// Completes a single-line statement and starts the next line as one undoable edit.
/// Unsupported or ambiguous syntax is left untouched for the language server.
pub(crate) fn complete(state: &mut EditorState) -> bool {
    if state.read_only
        || state.lang() != rc_syntax::Lang::CSharp
        || state.selection.len() != 1
        || !state.selection.primary().is_empty()
        || !state.syntax.is_current()
    {
        return false;
    }
    let rope = state.buffer.rope();
    let line = rope.char_to_line(state.caret());
    let line_start = rope.line_to_char(line);
    let raw = rope.line(line).to_string();
    let without_newline = raw.trim_end_matches(['\r', '\n']);
    if without_newline.chars().count() > 4096 {
        return false;
    }
    let content = without_newline.trim_end_matches([' ', '\t']);
    let statement = content.trim_start_matches([' ', '\t']);
    if statement.is_empty()
        || statement.starts_with(['#', '/', '*', '['])
        || statement.contains("//")
        || statement.contains("/*")
        || statement.contains("=>")
        || statement.contains(['{', '}'])
        || statement.ends_with(':')
    {
        return false;
    }
    let first = statement
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()
        .unwrap_or("");
    if matches!(
        first,
        "if" | "else"
            | "for"
            | "foreach"
            | "while"
            | "do"
            | "switch"
            | "try"
            | "catch"
            | "finally"
            | "using"
            | "lock"
            | "class"
            | "struct"
            | "interface"
            | "enum"
            | "namespace"
            | "public"
            | "private"
            | "protected"
            | "internal"
            | "static"
    ) {
        return false;
    }
    let last = statement.chars().last().unwrap();
    let has_semicolon = last == ';';
    if (first == "var" && !statement.contains(" = "))
        || (matches!(first, "throw" | "await") && statement == first)
    {
        return false;
    }
    if matches!(
        last,
        '=' | '+' | '-' | '*' | '/' | '.' | ',' | '(' | '[' | '?'
    ) || statement.chars().filter(|&c| c == '(').count()
        != statement.chars().filter(|&c| c == ')').count()
        || statement.chars().filter(|&c| c == '[').count()
            != statement.chars().filter(|&c| c == ']').count()
    {
        return false;
    }
    if !has_semicolon
        && !(matches!(
            first,
            "return" | "throw" | "var" | "await" | "break" | "continue"
        ) || last == ')'
            || (statement.contains(" = ")
                && (last.is_alphanumeric() || last == '"' || last == ']')))
    {
        return false;
    }
    // A current parse tree lets us reject lines inside multiline comments or strings.
    let probe = line_start + (without_newline.chars().count() - statement.chars().count());
    let byte = rope.char_to_byte(probe);
    let mut node = state
        .syntax
        .tree()
        .and_then(|tree| tree.root_node().descendant_for_byte_range(byte, byte));
    while let Some(current) = node {
        if current.kind().contains("comment") || current.kind().contains("string") {
            return false;
        }
        node = current.parent();
    }
    let end = line_start + without_newline.chars().count();
    let insert_at = line_start + content.chars().count();
    let base = commands::line_indent(rope, line);
    let replacement = format!("{}\n{base}", if has_semicolon { "" } else { ";" });
    let caret = insert_at + replacement.chars().count();
    let changes = ChangeSet::single(Change::new(insert_at, end, replacement));
    state.apply(
        Edited {
            changes,
            selection: Selection::point(caret),
        },
        EditKind::Other,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, EditorConfig};

    fn run(source: &str, caret: usize) -> EditorState {
        let mut state = EditorState::new(1, Some("App.cs".into()), source);
        state.selection = Selection::point(caret);
        state.parse_blocking();
        let context = egui::Context::default();
        let mut output = context.run_ui(Default::default(), |ui| {
            crate::run_action(
                ui,
                &mut state,
                &EditorConfig::default(),
                Action::CompleteStatement,
            );
        });
        output.textures_delta.clear();
        state
    }

    #[test]
    fn completes_return_and_moves_to_next_line_in_one_undo_step() {
        let source = "    return value  \n    next();";
        let mut state = run(source, 11);
        assert_eq!(
            state.buffer.rope().to_string(),
            "    return value;\n    \n    next();"
        );
        assert_eq!(state.caret_line_col(), (1, 4));
        state.undo();
        assert_eq!(state.buffer.rope().to_string(), source);
    }

    #[test]
    fn existing_semicolon_is_not_duplicated() {
        let source = "Console.WriteLine(1);";
        let state = run(source, source.chars().count());
        assert_eq!(state.buffer.rope().to_string(), "Console.WriteLine(1);\n");
    }

    #[test]
    fn ambiguous_lines_are_left_intact() {
        for source in [
            "if (ready)",
            "// return value",
            "var x =",
            "var x",
            "throw",
            "Call(",
            "/*\nreturn value\n*/",
            "#nullable enable",
        ] {
            let state = run(
                source,
                source
                    .chars()
                    .count()
                    .min(source.find("\n*/").unwrap_or(source.len())),
            );
            assert_eq!(state.buffer.rope().to_string(), source, "{source}");
        }
    }

    #[test]
    fn shortcut_and_guards() {
        use crate::{Keymap, KeymapKind};
        let shortcut = egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            Keymap::new(KeymapKind::Rider).lookup(egui::Key::Enter, shortcut),
            Some(Action::CompleteStatement)
        );
        let mut state = EditorState::new(2, Some("App.cs".into()), "return value");
        state.selection = Selection::point(state.buffer.len_chars());
        assert!(!complete(&mut state));
        state.parse_blocking();
        state.read_only = true;
        assert!(!complete(&mut state));
        assert_eq!(state.buffer.rope().to_string(), "return value");
    }
}
