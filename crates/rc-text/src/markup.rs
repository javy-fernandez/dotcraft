//! Typing assists for markup (HTML, Razor, XML): what to insert after `>` (the closing tag),
//! after `</` (the name of the open element) and after `=` in a tag (the attribute quotes).
//! Pure text functions, used by the editor right after the character is typed.

use crate::Rope;

/// HTML elements without a closing tag.
const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr", "!doctype"];

/// How far back to look for the tag being typed / the open elements.
const LOOK_BACK: usize = 20_000;

fn name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.')
}

/// The text from `pos - LOOK_BACK` to `pos`, and where it starts.
fn before(rope: &Rope, pos: usize) -> (Vec<char>, usize) {
    let start = pos.saturating_sub(LOOK_BACK);
    (rope.slice(start..pos).chars().collect(), start)
}

/// The tag that ends at the end of `text` (just typed `>`): its `<` index, name and whether it
/// is a closing tag. `None` for self-closing tags, comments, processing instructions, and `<`
/// that is not a tag (`List<Order>` in code, `a < b`).
fn tag_ending_here(text: &[char]) -> Option<(usize, String, bool)> {
    let end = text.len().checked_sub(1)?;
    if text[end] != '>' || (end > 0 && text[end - 1] == '/') {
        return None;
    }
    // Back to the `<`, skipping quoted attribute values.
    let mut i = end;
    let mut quote: Option<char> = None;
    while i > 0 {
        i -= 1;
        let c = text[i];
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '>' => return None,
            None if c == '<' => break,
            None => {}
        }
        if i == 0 {
            return None;
        }
    }
    if text[i] != '<' {
        return None;
    }
    let mut j = i + 1;
    let closing = text.get(j) == Some(&'/');
    // `List<Order>`: an opening `<` glued to an identifier is a generic, not a tag (`x</a>` is).
    if !closing && i > 0 && (text[i - 1].is_alphanumeric() || text[i - 1] == '_') {
        return None;
    }
    if closing {
        j += 1;
    }
    if !text.get(j).is_some_and(|c| c.is_alphabetic()) {
        return None;
    }
    let name: String = text[j..end].iter().take_while(|c| name_char(**c)).collect();
    Some((i, name, closing))
}

/// After typing `>`: the closing tag to insert at the caret (`</div>` for `<div class="x">`),
/// unless the element is void (HTML), self-closing, or already closed right after the caret.
pub fn close_tag(rope: &Rope, caret: usize, html: bool) -> Option<String> {
    let (text, _) = before(rope, caret);
    let (_, name, closing) = tag_ending_here(&text)?;
    if closing || name.is_empty() || (html && VOID.contains(&name.to_ascii_lowercase().as_str())) {
        return None;
    }
    let closing_tag = format!("</{name}>");
    let after: String = rope.slice(caret..(caret + closing_tag.chars().count()).min(rope.len_chars())).chars().collect();
    (after != closing_tag).then_some(closing_tag)
}

/// After typing `</`: the name of the innermost element still open before it, and `>`.
pub fn close_open_element(rope: &Rope, caret: usize, html: bool) -> Option<String> {
    let (text, _) = before(rope, caret);
    if text.len() < 2 || text[text.len() - 2..] != ['<', '/'] {
        return None;
    }
    let body = &text[..text.len() - 2];
    // Walk the tags forwards with a stack of open elements.
    let mut stack: Vec<String> = Vec::new();
    let mut i = 0;
    while i < body.len() {
        if body[i] == '<' {
            // Comments are skipped whole.
            if body[i..].starts_with(&['<', '!', '-', '-']) {
                let rest: String = body[i..].iter().collect();
                i += rest.find("-->").map_or(body.len(), |k| k + 3);
                continue;
            }
            // Find this tag's `>`.
            let mut j = i + 1;
            let mut quote: Option<char> = None;
            while j < body.len() {
                let c = body[j];
                match quote {
                    Some(q) if c == q => quote = None,
                    Some(_) => {}
                    None if c == '"' || c == '\'' => quote = Some(c),
                    None if c == '>' || c == '<' => break,
                    None => {}
                }
                j += 1;
            }
            if j < body.len() && body[j] == '>'
                && let Some((_, name, closing)) = tag_ending_here(&body[..=j])
            {
                let self_closing = body[j - 1] == '/';
                if closing {
                    if let Some(k) = stack.iter().rposition(|n| *n == name) {
                        stack.truncate(k);
                    }
                } else if !self_closing && !(html && VOID.contains(&name.to_ascii_lowercase().as_str())) {
                    stack.push(name);
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    stack.pop().map(|n| format!("{n}>"))
}

/// After typing `=` right after an attribute name inside a tag: whether to add `""` (and put the
/// caret between them).
pub fn quote_attribute(rope: &Rope, caret: usize) -> bool {
    let (text, _) = before(rope, caret);
    let Some(eq) = text.len().checked_sub(1) else { return false };
    if text[eq] != '=' || eq == 0 || !name_char(text[eq - 1]) {
        return false;
    }
    if rope.get_char(caret).is_some_and(|c| c == '"' || c == '\'') {
        return false;
    }
    // Inside an open tag: a `<name` before, with no `>` in between (quotes balanced).
    let mut quote: Option<char> = None;
    let mut i = eq;
    while i > 0 {
        i -= 1;
        let c = text[i];
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '>' => return false,
            None if c == '<' => return text.get(i + 1).is_some_and(|c| c.is_alphabetic()) && (i == 0 || !(text[i - 1].is_alphanumeric() || text[i - 1] == '_')),
            None => {}
        }
    }
    false
}

/// Whether the caret sits between an opening tag and its closing tag (`<div>|</div>`), where
/// Enter opens an indented block.
pub fn between_tags(rope: &Rope, caret: usize) -> bool {
    let (text, _) = before(rope, caret);
    let Some((_, name, closing)) = tag_ending_here(&text) else { return false };
    if closing {
        return false;
    }
    let want = format!("</{name}>");
    let after: String = rope.slice(caret..(caret + want.chars().count()).min(rope.len_chars())).chars().collect();
    after == want
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> (Rope, usize) {
        let caret = text.find('|').unwrap();
        let s = text.replacen('|', "", 1);
        (Rope::from_str(&s), s[..caret].chars().count())
    }

    #[test]
    fn closes_tags_but_not_void_self_closing_or_generics() {
        let (r, c) = at("<div class=\"a>b\">|");
        assert_eq!(close_tag(&r, c, true).as_deref(), Some("</div>"));
        let (r, c) = at("<br>|");
        assert_eq!(close_tag(&r, c, true), None);
        let (r, c) = at("<PropertyGroup>|");
        assert_eq!(close_tag(&r, c, false).as_deref(), Some("</PropertyGroup>"));
        let (r, c) = at("<img src=\"x\" />|");
        assert_eq!(close_tag(&r, c, true), None);
        let (r, c) = at("List<Order>|");
        assert_eq!(close_tag(&r, c, true), None);
        let (r, c) = at("</div>|");
        assert_eq!(close_tag(&r, c, true), None);
        let (r, c) = at("<p>|</p>");
        assert_eq!(close_tag(&r, c, true), None);
        let (r, c) = at("if (a <b>|");
        assert_eq!(close_tag(&r, c, true).as_deref(), Some("</b>"), "a space before `<` reads as a tag");
    }

    #[test]
    fn completes_the_innermost_open_element() {
        let (r, c) = at("<ul>\n  <li><a href=\"#\">x</a><br>\n  </|");
        assert_eq!(close_open_element(&r, c, true).as_deref(), Some("li>"));
        let (r, c) = at("<ul><li>a</li><!-- <p> --></|");
        assert_eq!(close_open_element(&r, c, true).as_deref(), Some("ul>"));
        let (r, c) = at("text </|");
        assert_eq!(close_open_element(&r, c, true), None);
    }

    #[test]
    fn quotes_attributes_inside_tags_only() {
        let (r, c) = at("<div class=|");
        assert!(quote_attribute(&r, c));
        let (r, c) = at("var x =|");
        assert!(!quote_attribute(&r, c));
        let (r, c) = at("<div>a=|");
        assert!(!quote_attribute(&r, c));
        let (r, c) = at("<div title=\"a=b\" id=|");
        assert!(quote_attribute(&r, c));
    }

    #[test]
    fn detects_the_caret_between_a_pair_of_tags() {
        let (r, c) = at("<div class=\"x\">|</div>");
        assert!(between_tags(&r, c));
        let (r, c) = at("<div>|</span>");
        assert!(!between_tags(&r, c));
    }
}
