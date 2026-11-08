use std::path::Path;

use rc_syntax::{Hl, Lang, folding_ranges, highlight_lines, parse_now};
use rc_text::Rope;

#[test]
fn web_files_have_native_grammars_highlights_and_folds() {
    for (file, lang, source, token, role) in [
        ("index.HTML", Lang::Html, "<main class=\"page\">\n  <p>Hello</p>\n</main>\n", "main", Hl::Tag),
        ("app.css", Lang::Css, ".page {\n  color: #ffffff;\n}\n", "color", Hl::Property),
        ("app.mjs", Lang::JavaScript, "function greet() {\n  return true;\n}\n", "function", Hl::Keyword),
        ("app.ts", Lang::TypeScript, "interface Person {\n  name: string;\n}\n", "interface", Hl::Keyword),
        ("app.tsx", Lang::Tsx, "function View() {\n  return <main>Hello</main>;\n}\n", "return", Hl::Keyword),
    ] {
        assert_eq!(Lang::from_path(Path::new(file)), lang);
        let rope = Rope::from_str(source);
        let tree = parse_now(lang, &rope, None).unwrap();
        assert!(!tree.root_node().has_error(), "{file}: {}", tree.root_node().to_sexp());
        let spans = highlight_lines(lang, &tree, &rope, 0, rope.len_lines());
        assert!(spans.iter().enumerate().any(|(line, spans)| spans.iter().any(|span| {
            span.hl == role && rope.line(line).chars().skip(span.start).take(span.end - span.start).collect::<String>() == token
        })), "{file}: missing {role:?} for {token}");
        assert!(!folding_ranges(lang, &tree, &rope).is_empty(), "{file}: missing fold");
    }
}

#[test]
fn web_comments_and_razor_detection_remain_distinct() {
    for ext in ["js", "mjs", "cjs", "jsx"] {
        assert_eq!(Lang::from_path(Path::new(&format!("app.{ext}"))), Lang::JavaScript);
    }
    for ext in ["ts", "mts", "cts"] {
        assert_eq!(Lang::from_path(Path::new(&format!("app.{ext}"))), Lang::TypeScript);
    }
    for ext in ["razor", "cshtml"] {
        assert_eq!(Lang::from_path(Path::new(&format!("view.{ext}"))), Lang::Razor);
    }
    assert_eq!(Lang::Html.block_comment(), Some(("<!--", "-->")));
    assert_eq!(Lang::Html.line_comment(), None);
    assert_eq!(Lang::Css.line_comment(), None);
    for lang in [Lang::JavaScript, Lang::TypeScript, Lang::Tsx] {
        assert_eq!(lang.line_comment(), Some("//"));
        assert_eq!(lang.block_comment(), Some(("/*", "*/")));
    }
}

#[test]
fn jsx_and_tsx_highlight_components_attributes_comments_and_constants() {
    for (file, lang) in [("view.jsx", Lang::JavaScript), ("view.tsx", Lang::Tsx)] {
        let source = "const view = <Panel title=\"Hello\" enabled={true}>\n  {/* explanation */}\n  <span data-kind=\"item\" />\n  <UI.Badge count={42} />\n</Panel>;\n";
        let rope = Rope::from_str(source);
        let tree = parse_now(lang, &rope, None).unwrap();
        assert!(!tree.root_node().has_error(), "{file}: {}", tree.root_node().to_sexp());
        let folds = folding_ranges(lang, &tree, &rope);
        assert!(folds.iter().any(|fold| fold.start_line == 0 && fold.end_line == 4), "{file}: missing multiline component fold");
        let highlights = highlight_lines(lang, &tree, &rope, 0, rope.len_lines());
        for (token, role) in [
            ("Panel", Hl::Tag),
            ("span", Hl::Tag),
            ("Badge", Hl::Tag),
            ("title", Hl::XmlAttr),
            ("enabled", Hl::XmlAttr),
            ("data-kind", Hl::XmlAttr),
            ("count", Hl::XmlAttr),
            ("\"Hello\"", Hl::String),
            ("true", Hl::Constant),
            ("42", Hl::Number),
            ("/* explanation */", Hl::Comment),
        ] {
            assert!(highlights.iter().enumerate().any(|(line, spans)| spans.iter().any(|span| {
                span.hl == role && rope.line(line).chars().skip(span.start).take(span.end - span.start).collect::<String>() == token
            })), "{file}: missing {role:?} for {token}");
        }
        assert_eq!(highlights.iter().enumerate().map(|(line, spans)| spans.iter().filter(|span| {
            span.hl == Hl::Tag && rope.line(line).chars().skip(span.start).take(span.end - span.start).collect::<String>() == "Panel"
        }).count()).sum::<usize>(), 2, "both component tags must be styled in {file}");
    }
}
