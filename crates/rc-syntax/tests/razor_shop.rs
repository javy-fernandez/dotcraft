use rc_syntax::{Hl, Lang, folding_ranges, highlight_lines, parse_now};
use rc_text::Rope;
use std::path::Path;

#[test]
fn shop_order_component_parses_razor_markup_and_csharp() {
    let source = "@page \"/orders\"\n@using Shop.Core\n<h1>Orders</h1>\n@if (selected is not null)\n{\n    <p>@selected.Customer</p>\n}\n@code {\n    private Order? selected;\n}\n";
    assert_eq!(Lang::from_path(Path::new("Orders.razor")), Lang::Razor);
    let rope = Rope::from_str(source);
    let tree = parse_now(Lang::Razor, &rope, None).unwrap();
    assert!(!tree.root_node().has_error(), "{}", tree.root_node().to_sexp());
    let spans = highlight_lines(Lang::Razor, &tree, &rope, 0, rope.len_lines());
    let has = |token: &str, role: Hl| spans.iter().enumerate().any(|(line, spans)| spans.iter().any(|span| {
        span.hl == role && rope.line(line).chars().skip(span.start).take(span.end - span.start).collect::<String>() == token
    }));
    assert!(has("@page", Hl::Keyword));
    assert!(has("<h1>", Hl::Tag));
    assert!(has("<p>", Hl::Tag));
    assert!(has("Order", Hl::Class));
    assert!(folding_ranges(Lang::Razor, &tree, &rope).iter().any(|fold| fold.start_line == 7 && fold.end_line == 9));
}
