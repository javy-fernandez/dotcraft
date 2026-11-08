//! Snippets with tab stops (LSP snippet syntax, used by completion items and live templates):
//! after inserting one, Tab and Shift+Tab move between its fields, and every occurrence of a
//! field is edited at once (one caret per occurrence).

/// A parsed snippet: the plain text and its fields, by number (`0` is the final caret).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    pub text: String,
    /// (number, char ranges in `text`).
    pub fields: Vec<(u32, Vec<(usize, usize)>)>,
}

/// Parses `$1`, `${1}`, `${1:placeholder}` (nested fields too), `${1|a,b|}` (the first choice)
/// and the `\$`, `\}`, `\\` escapes.
pub fn parse(s: &str) -> Parsed {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut fields: Vec<(u32, Vec<(usize, usize)>)> = Vec::new();
    parse_into(&chars, &mut 0, &mut out, &mut fields, false);
    fields.sort_by_key(|(n, _)| if *n == 0 { u32::MAX } else { *n });
    Parsed { text: out, fields }
}

fn add(fields: &mut Vec<(u32, Vec<(usize, usize)>)>, n: u32, range: (usize, usize)) {
    match fields.iter_mut().find(|(m, _)| *m == n) {
        Some((_, r)) => r.push(range),
        None => fields.push((n, vec![range])),
    }
}

/// A bare `$n` repeats the text an earlier `${n:text}` gave (a mirror).
fn mirror(out: &mut String, fields: &mut Vec<(u32, Vec<(usize, usize)>)>, n: u32) {
    let at = out.chars().count();
    let text: String = fields.iter().find(|(m, _)| *m == n).and_then(|(_, r)| r.first()).map(|&(a, b)| out.chars().skip(a).take(b - a).collect()).unwrap_or_default();
    out.push_str(&text);
    add(fields, n, (at, at + text.chars().count()));
}

fn number(chars: &[char], i: &mut usize) -> Option<u32> {
    let start = *i;
    let mut n = 0u32;
    while *i < chars.len() && chars[*i].is_ascii_digit() {
        n = n.saturating_mul(10).saturating_add(chars[*i].to_digit(10).unwrap_or(0));
        *i += 1;
    }
    (*i > start).then_some(n)
}

/// Parses until the end (or the `}` closing a placeholder when `nested`).
fn parse_into(chars: &[char], i: &mut usize, out: &mut String, fields: &mut Vec<(u32, Vec<(usize, usize)>)>, nested: bool) {
    let len = |o: &String| o.chars().count();
    while *i < chars.len() {
        let c = chars[*i];
        if c == '\\' && *i + 1 < chars.len() && matches!(chars[*i + 1], '$' | '}' | '\\' | ',' | '|') {
            out.push(chars[*i + 1]);
            *i += 2;
            continue;
        }
        if nested && c == '}' {
            *i += 1;
            return;
        }
        if c == '$' && *i + 1 < chars.len() {
            let mut j = *i + 1;
            if let Some(n) = number(chars, &mut j) {
                mirror(out, fields, n);
                *i = j;
                continue;
            }
            if chars[j] == '{' {
                j += 1;
                if let Some(n) = number(chars, &mut j) {
                    let at = len(out);
                    match chars.get(j) {
                        Some('}') => {
                            mirror(out, fields, n);
                            *i = j + 1;
                        }
                        Some(':') => {
                            *i = j + 1;
                            parse_into(chars, i, out, fields, true);
                            add(fields, n, (at, len(out)));
                        }
                        Some('|') => {
                            // Choice: the first option.
                            let mut k = j + 1;
                            let mut first = String::new();
                            while k < chars.len() && chars[k] != ',' && chars[k] != '|' {
                                first.push(chars[k]);
                                k += 1;
                            }
                            while k < chars.len() && chars[k] != '}' {
                                k += 1;
                            }
                            out.push_str(&first);
                            add(fields, n, (at, len(out)));
                            *i = k + 1;
                        }
                        _ => {
                            out.push(c);
                            *i += 1;
                        }
                    }
                    continue;
                }
                // `${NAME}` / `${NAME:default}` variables: the default, or nothing.
                let mut k = j;
                while k < chars.len() && (chars[k].is_ascii_alphanumeric() || chars[k] == '_') {
                    k += 1;
                }
                if k > j && k < chars.len() && (chars[k] == '}' || chars[k] == ':') {
                    if chars[k] == ':' {
                        *i = k + 1;
                        parse_into(chars, i, out, fields, true);
                    } else {
                        *i = k + 1;
                    }
                    continue;
                }
            }
        }
        out.push(c);
        *i += 1;
    }
}

/// Fields of an inserted snippet, in absolute char offsets, in Tab order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub fields: Vec<Vec<(usize, usize)>>,
    pub current: usize,
}

impl Session {
    /// The ranges the caret may be in while the session lasts.
    pub fn contains(&self, pos: usize) -> bool {
        self.fields.iter().flatten().any(|&(a, b)| (a..=b).contains(&pos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fields_placeholders_and_mirrors() {
        let p = parse("for (int ${1:i} = 0; $1 < ${2:n}; $1++)\n{\n\t$0\n}");
        assert_eq!(p.text, "for (int i = 0; i < n; i++)\n{\n\t\n}");
        assert_eq!(p.fields[0], (1, vec![(9, 10), (16, 17), (23, 24)]));
        assert_eq!(p.fields[1], (2, vec![(20, 21)]));
        assert_eq!(p.fields[2].0, 0, "the final caret goes last");
    }

    #[test]
    fn nested_choices_variables_and_escapes() {
        let p = parse("${1:public ${2:int}} \\$x ${3|get,set|} ${TM_FILENAME_BASE:Name}");
        assert_eq!(p.text, "public int $x get Name");
        assert_eq!(p.fields[0], (1, vec![(0, 10)]));
        assert_eq!(p.fields[1], (2, vec![(7, 10)]));
        assert_eq!(p.fields[2], (3, vec![(14, 17)]));
    }
}
