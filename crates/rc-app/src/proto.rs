//! Lightweight `.proto` declarations for the Structure window and gRPC method selection.

use rc_lsp::lsp_types::SymbolKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub line: usize,
    pub col: usize,
    pub depth: usize,
}

/// Reads declarations without invoking a language server. Comments and quoted strings are
/// masked before looking for `message`, `enum`, `service`, `oneof` and `rpc`.
pub fn symbols(source: &str) -> Vec<Symbol> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut in_block_comment = false;
    for (line, original) in source.lines().enumerate() {
        let mut clean = String::with_capacity(original.len());
        let bytes = original.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if in_block_comment {
                if bytes[i..].starts_with(b"*/") {
                    in_block_comment = false;
                    clean.push_str("  ");
                    i += 2;
                } else {
                    clean.push(' ');
                    i += 1;
                }
            } else if bytes[i..].starts_with(b"//") {
                clean.extend(std::iter::repeat_n(' ', bytes.len() - i));
                break;
            } else if bytes[i..].starts_with(b"/*") {
                in_block_comment = true;
                clean.push_str("  ");
                i += 2;
            } else if bytes[i] == b'"' || bytes[i] == b'\'' {
                let quote = bytes[i];
                clean.push(' ');
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        clean.push(' ');
                        i += 1;
                        if i < bytes.len() {
                            clean.push(' ');
                            i += 1;
                        }
                        continue;
                    }
                    clean.push(' ');
                    i += 1;
                    if bytes[i - 1] == quote {
                        break;
                    }
                }
            } else {
                clean.push(if bytes[i].is_ascii() {
                    bytes[i] as char
                } else {
                    ' '
                });
                i += 1;
            }
        }
        let words: Vec<(usize, &str)> = clean
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .filter(|word| !word.is_empty())
            .scan(0usize, |offset, word| {
                let start = clean[*offset..].find(word)? + *offset;
                *offset = start + word.len();
                Some((start, word))
            })
            .collect();
        for pair in words.windows(2) {
            let (col, keyword) = pair[0];
            let (_, name) = pair[1];
            let kind = match keyword {
                "message" => Some(SymbolKind::STRUCT),
                "enum" => Some(SymbolKind::ENUM),
                "service" => Some(SymbolKind::INTERFACE),
                "oneof" => Some(SymbolKind::ENUM),
                "rpc" => Some(SymbolKind::METHOD),
                "package" => Some(SymbolKind::PACKAGE),
                _ => None,
            };
            if let Some(kind) = kind {
                let name = if kind == SymbolKind::PACKAGE {
                    clean[col + keyword.len()..]
                        .trim()
                        .trim_end_matches(';')
                        .trim()
                        .to_string()
                } else {
                    name.to_string()
                };
                let local_depth = clean[..col].chars().fold(depth, |level, ch| match ch {
                    '{' => level + 1,
                    '}' => level.saturating_sub(1),
                    _ => level,
                });
                result.push(Symbol {
                    name,
                    kind,
                    line,
                    col: original[..col].chars().count(),
                    depth: local_depth,
                });
            }
        }
        for ch in clean.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    result
}

/// Fully qualified method names accepted by grpcurl (`package.Service/Method`).
pub fn methods(source: &str) -> Vec<String> {
    let symbols = symbols(source);
    let package = symbols
        .iter()
        .find(|symbol| symbol.kind == SymbolKind::PACKAGE)
        .map(|symbol| symbol.name.clone())
        .unwrap_or_default();
    let mut service: Option<(String, usize)> = None;
    let mut result = Vec::new();
    for symbol in symbols {
        if symbol.kind == SymbolKind::INTERFACE {
            service = Some((symbol.name, symbol.depth));
        } else if symbol.kind == SymbolKind::METHOD {
            if let Some((name, depth)) = &service
                && symbol.depth > *depth
            {
                let prefix = if package.is_empty() {
                    name.clone()
                } else {
                    format!("{package}.{name}")
                };
                result.push(format!("{prefix}/{}", symbol.name));
            }
        } else if service
            .as_ref()
            .is_some_and(|(_, depth)| symbol.depth <= *depth)
        {
            service = None;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_declarations_and_ignores_comments_and_strings() {
        let symbols = symbols(
            "// message Fake {}\nservice Greeter {\n  rpc SayHello (Hello) returns (Reply);\n}\nmessage Hello {\n  string value = 1;\n}\noption note = \"service Hidden {}\";\n",
        );
        assert_eq!(
            symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["Greeter", "SayHello", "Hello"]
        );
        assert_eq!(symbols[1].depth, 1);
        assert_eq!(symbols[1].line, 2);
    }

    #[test]
    fn qualifies_service_methods() {
        let source = "syntax = \"proto3\";\n/* package wrong; */\npackage shop.v1;\nservice Catalog {\n  rpc Get (Request) returns (Reply);\n  rpc List (Request) returns (stream Reply);\n}\n";
        assert_eq!(
            methods(source),
            ["shop.v1.Catalog/Get", "shop.v1.Catalog/List"]
        );
    }
}
