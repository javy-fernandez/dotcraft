//! Conversions between LSP types and the editor's.

use rc_editor::{CompletionItem, Diagnostic, Severity};
use rc_kit::{Ico, ico};
use rc_lsp::lsp_types as lsp;
use rc_text::{Rope, Utf16Pos};

pub fn char_of(rope: &Rope, p: lsp::Position) -> usize {
    rc_text::buffer::utf16_to_char(rope, Utf16Pos { line: p.line, character: p.character })
}

pub fn lsp_position(rope: &Rope, pos: usize) -> lsp::Position {
    let p = rc_text::buffer::char_to_utf16(rope, pos);
    lsp::Position { line: p.line, character: p.character }
}

pub fn range_chars(rope: &Rope, r: &lsp::Range) -> (usize, usize) {
    (char_of(rope, r.start), char_of(rope, r.end))
}

pub fn diagnostic(rope: &Rope, d: &lsp::Diagnostic) -> Diagnostic {
    let (start, end) = range_chars(rope, &d.range);
    let severity = match d.severity {
        Some(lsp::DiagnosticSeverity::ERROR) => Severity::Error,
        Some(lsp::DiagnosticSeverity::WARNING) => Severity::Warning,
        Some(lsp::DiagnosticSeverity::INFORMATION) => Severity::Info,
        _ => Severity::Hint,
    };
    let tags = d.tags.clone().unwrap_or_default();
    let code = d.code.as_ref().map(|c| match c {
        lsp::NumberOrString::Number(n) => n.to_string(),
        lsp::NumberOrString::String(s) => s.clone(),
    });
    // Roslyn reports IDE0005 once over the whole block of usings; only a single directive
    // can be faded reliably (the block also holds the necessary ones).
    let block = code.as_deref() == Some("IDE0005") && d.range.end.line > d.range.start.line;
    Diagnostic {
        start,
        end,
        severity,
        message: d.message.clone(),
        code,
        unnecessary: tags.contains(&lsp::DiagnosticTag::UNNECESSARY) && !block,
        deprecated: tags.contains(&lsp::DiagnosticTag::DEPRECATED),
    }
}

pub fn completion_icon(kind: Option<lsp::CompletionItemKind>) -> Ico {
    use lsp::CompletionItemKind as K;
    match kind {
        Some(K::CLASS) => ico::CLASS,
        Some(K::INTERFACE) => ico::INTERFACE,
        Some(K::STRUCT) => ico::STRUCT,
        Some(K::ENUM) => ico::ENUM,
        Some(K::ENUM_MEMBER) => ico::CONSTANT,
        Some(K::METHOD) | Some(K::FUNCTION) => ico::METHOD,
        Some(K::CONSTRUCTOR) => ico::CONSTRUCTOR,
        Some(K::FIELD) => ico::FIELD,
        Some(K::PROPERTY) => ico::PROPERTY,
        Some(K::EVENT) => ico::EVENT,
        Some(K::CONSTANT) => ico::CONSTANT,
        Some(K::VARIABLE) => ico::VARIABLE,
        Some(K::MODULE) => ico::NAMESPACE,
        Some(K::KEYWORD) => ico::KEYWORD,
        Some(K::SNIPPET) => ico::TEMPLATE,
        Some(K::TYPE_PARAMETER) => ico::TYPE,
        Some(K::OPERATOR) => ico::FUNCTION,
        Some(K::REFERENCE) | Some(K::FILE) | Some(K::FOLDER) => ico::FILE_TEXT,
        _ => ico::TYPE,
    }
}

/// An editor completion item from an LSP one. `default_range` comes from the list's
/// `itemDefaults.editRange`.
pub fn completion_item(rope: &Rope, item: &lsp::CompletionItem, index: usize, default_range: Option<&lsp::Range>) -> CompletionItem {
    let (insert, replace) = match &item.text_edit {
        Some(lsp::CompletionTextEdit::Edit(e)) => (e.new_text.clone(), Some(range_chars(rope, &e.range))),
        Some(lsp::CompletionTextEdit::InsertAndReplace(e)) => (e.new_text.clone(), Some(range_chars(rope, &e.replace))),
        None => (
            item.insert_text.clone().unwrap_or_else(|| item.label.clone()),
            default_range.map(|r| range_chars(rope, r)),
        ),
    };
    let snippet = item.insert_text_format == Some(lsp::InsertTextFormat::SNIPPET);
    let insert = if snippet { insert } else { insert.replace('$', "\\$") };
    let details = item.label_details.as_ref();
    CompletionItem {
        label: item.label.clone(),
        detail: details.and_then(|d| d.detail.clone()).unwrap_or_default(),
        type_text: details.and_then(|d| d.description.clone()).or_else(|| item.detail.clone()).unwrap_or_default(),
        icon: completion_icon(item.kind),
        insert,
        replace,
        deprecated: item.deprecated.unwrap_or(false) || item.tags.as_ref().is_some_and(|t| t.contains(&lsp::CompletionItemTag::DEPRECATED)),
        data: index,
    }
}

pub fn markup_text(m: &lsp::HoverContents) -> String {
    match m {
        lsp::HoverContents::Markup(m) => m.value.clone(),
        lsp::HoverContents::Scalar(s) => marked(s),
        lsp::HoverContents::Array(a) => a.iter().map(marked).collect::<Vec<_>>().join("\n\n"),
    }
}

fn marked(s: &lsp::MarkedString) -> String {
    match s {
        lsp::MarkedString::String(s) => s.clone(),
        lsp::MarkedString::LanguageString(l) => format!("```{}\n{}\n```", l.language, l.value),
    }
}

pub fn documentation(d: &lsp::Documentation) -> String {
    match d {
        lsp::Documentation::String(s) => s.clone(),
        lsp::Documentation::MarkupContent(m) => m.value.clone(),
    }
}

pub fn symbol_icon(kind: lsp::SymbolKind) -> Ico {
    use lsp::SymbolKind as K;
    match kind {
        K::CLASS => ico::CLASS,
        K::INTERFACE => ico::INTERFACE,
        K::STRUCT => ico::STRUCT,
        K::ENUM => ico::ENUM,
        K::ENUM_MEMBER => ico::CONSTANT,
        K::METHOD | K::FUNCTION => ico::METHOD,
        K::CONSTRUCTOR => ico::CONSTRUCTOR,
        K::FIELD => ico::FIELD,
        K::PROPERTY => ico::PROPERTY,
        K::EVENT => ico::EVENT,
        K::CONSTANT => ico::CONSTANT,
        K::NAMESPACE | K::PACKAGE | K::MODULE => ico::NAMESPACE,
        K::VARIABLE => ico::VARIABLE,
        K::TYPE_PARAMETER => ico::TYPE,
        K::OPERATOR => ico::FUNCTION,
        _ => ico::TYPE,
    }
}
