//! Evaluation without side effects: whether showing an expression's value may run code in the
//! program.
//!
//! netcoredbg's engine can skip function evaluation (`EVAL_NOFUNCEVAL`), but its DAP front end
//! never sets it: `evaluate`, `variables` and `setExpression` always run with the default flags
//! (src/protocols/vscodeprotocol.cpp: "VSCode don't support evaluation flags, we can't disable
//! implicit function calls during evaluation"). Every property read in an expression and every
//! object expanded in the Variables tree runs getters in the debuggee. DotCraft therefore
//! decides before asking: under a restrictive [`EvalPolicy`] an expression is sent only when
//! [`side_effects`] proves it calls nothing, and objects are expanded only on request.

use std::collections::HashMap;

/// What evaluations from hovers, watches and the Variables tree may do (Rider's "Allow property
/// evaluation and other implicit function calls").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvalPolicy {
    /// Property getters, indexers and user-defined operators or conversions; also listing the
    /// members of an object, which runs all of its getters.
    pub implicit_calls: bool,
    /// Method calls, object creation, lambdas and assignments written in a watch.
    pub method_calls: bool,
}

impl Default for EvalPolicy {
    fn default() -> Self {
        EvalPolicy { implicit_calls: true, method_calls: true }
    }
}

impl EvalPolicy {
    pub fn unrestricted(self) -> bool {
        self.implicit_calls && self.method_calls
    }

    pub fn permits(self, effects: SideEffects) -> bool {
        (self.implicit_calls || !effects.implicit_calls) && (self.method_calls || !effects.method_calls)
    }
}

/// Code an expression may run when evaluated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SideEffects {
    /// A member access (field or property: the debugger cannot tell), an indexer, an
    /// identifier that is not a local, a cast or an operator on a non-primitive value.
    pub implicit_calls: bool,
    /// A call, `new`, a lambda, an assignment or an increment.
    pub method_calls: bool,
}

impl SideEffects {
    pub fn none(self) -> bool {
        !self.implicit_calls && !self.method_calls
    }
}

/// Types whose operators netcoredbg computes itself (`CalculationDelegate`) instead of calling
/// an `op_*` method of the type (`decimal`, nullable values and structs call one).
const PRIMITIVE: [&str; 13] = ["bool", "byte", "sbyte", "char", "short", "ushort", "int", "uint", "long", "ulong", "float", "double", "string"];

/// What evaluating `expression` may run, given the locals and arguments of the frame (and
/// `this`) with their types as netcoredbg prints them. Anything it cannot prove harmless counts:
/// `order.Total` may be a property, `Total` alone may be a property of `this`.
pub fn side_effects(expression: &str, locals: &HashMap<String, String>) -> SideEffects {
    let mut fx = SideEffects::default();
    let tokens = tokenize(expression);
    let mut operators = false;
    let mut operand_types: Vec<&str> = Vec::new();
    // Open parentheses: true for a call's argument list, false for grouping (or a cast).
    let mut parens: Vec<bool> = Vec::new();
    for (i, tok) in tokens.iter().enumerate() {
        let prev = i.checked_sub(1).map(|p| &tokens[p]);
        let next = tokens.get(i + 1);
        match tok {
            Tok::Bad => {
                fx.implicit_calls = true;
                fx.method_calls = true;
            }
            Tok::Literal => operand_types.push("int"),
            Tok::Ident(name) => match name.as_str() {
                "true" | "false" | "null" => {}
                "is" | "as" => {}
                "new" | "await" | "stackalloc" | "delegate" | "checked" | "unchecked" | "typeof" | "sizeof" | "default" | "nameof" => fx.method_calls = true,
                // The type after `is` / `as` is not evaluated.
                _ if matches!(prev, Some(Tok::Ident(k)) if k == "is" || k == "as") => {}
                _ => {
                    let member = matches!(prev, Some(Tok::Punct(p)) if matches!(p.as_str(), "." | "?." | "->" | "::"));
                    if next.is_some_and(|n| n.is("(")) {
                        fx.method_calls = true;
                    } else if member {
                        fx.implicit_calls = true;
                        operand_types.push("");
                    } else if name == "this" {
                        operand_types.push("");
                    } else if let Some(ty) = locals.get(name.as_str()) {
                        operand_types.push(ty);
                    } else {
                        fx.implicit_calls = true;
                        operand_types.push("");
                    }
                }
            },
            Tok::Punct(p) => match p.as_str() {
                "(" => {
                    // After a name, `)`, `]` or `>` (generic arguments) it is an invocation.
                    let call = matches!(prev, Some(Tok::Ident(_))) || prev.is_some_and(|t| t.is(")") || t.is("]") || t.is(">"));
                    if call {
                        fx.method_calls = true;
                    }
                    parens.push(call);
                }
                ")" => {
                    // `(Type)value`: a cast may call a user-defined conversion.
                    if parens.pop() == Some(false) && next.is_some_and(|n| matches!(n, Tok::Ident(_) | Tok::Literal) || n.is("(")) {
                        fx.implicit_calls = true;
                    }
                }
                "[" => {
                    // Only the elements of an array local are read without calling an indexer.
                    let array = match (i.checked_sub(2).map(|p| &tokens[p]), prev) {
                        (before, Some(Tok::Ident(name))) => {
                            !before.is_some_and(|b| b.is(".") || b.is("?.")) && locals.get(name.as_str()).is_some_and(|t| t.ends_with(']'))
                        }
                        _ => false,
                    };
                    if !array {
                        fx.implicit_calls = true;
                    }
                }
                "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | "??=" | "++" | "--" | "=>" => fx.method_calls = true,
                "+" | "-" | "*" | "/" | "%" | "<" | ">" | "<=" | ">=" | "==" | "!=" | "&&" | "||" | "&" | "|" | "^" | "!" | "~" | "<<" | ">>" => operators = true,
                "->" | "::" => fx.implicit_calls = true,
                "." | "?." | "?" | ":" | "??" | "," | "]" => {}
                _ => {
                    fx.implicit_calls = true;
                    fx.method_calls = true;
                }
            },
        }
    }
    if operators && operand_types.iter().any(|t| !PRIMITIVE.contains(t)) {
        fx.implicit_calls = true;
    }
    fx
}

#[derive(Debug, PartialEq)]
enum Tok {
    Ident(String),
    Literal,
    Punct(String),
    /// Something not understood (interpolated strings, braces, unterminated literals…).
    Bad,
}

impl Tok {
    fn is(&self, p: &str) -> bool {
        matches!(self, Tok::Punct(q) if q == p)
    }
}

const PUNCT3: [&str; 3] = ["??=", "<<=", ">>="];
const PUNCT2: [&str; 21] = ["?.", "??", "==", "!=", "<=", ">=", "&&", "||", "=>", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "->", "::"];

fn tokenize(text: &str) -> Vec<Tok> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let ident_char = |c: char| c.is_alphanumeric() || c == '_';
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '"' || (c == '@' && chars.get(i + 1) == Some(&'"')) {
            let verbatim = c == '@';
            i += if verbatim { 2 } else { 1 };
            let mut closed = false;
            while i < chars.len() {
                match chars[i] {
                    '\\' if !verbatim => i += 2,
                    '"' if verbatim && chars.get(i + 1) == Some(&'"') => i += 2,
                    '"' => {
                        closed = true;
                        i += 1;
                        break;
                    }
                    _ => i += 1,
                }
            }
            out.push(if closed { Tok::Literal } else { Tok::Bad });
        } else if c == '\'' {
            i += 1;
            let mut closed = false;
            while i < chars.len() {
                match chars[i] {
                    '\\' => i += 2,
                    '\'' => {
                        closed = true;
                        i += 1;
                        break;
                    }
                    _ => i += 1,
                }
            }
            out.push(if closed { Tok::Literal } else { Tok::Bad });
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            while i < chars.len() && (ident_char(chars[i]) || (chars[i] == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))) {
                i += 1;
            }
            out.push(Tok::Literal);
        } else if ident_char(c) || (c == '@' && chars.get(i + 1).is_some_and(|&n| ident_char(n))) {
            let start = if c == '@' { i + 1 } else { i };
            i = start;
            while i < chars.len() && ident_char(chars[i]) {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
        } else {
            let rest: String = chars[i..chars.len().min(i + 3)].iter().collect();
            let p = PUNCT3.iter().chain(PUNCT2.iter()).find(|p| rest.starts_with(**p)).map(|p| p.to_string());
            match p {
                Some(p) => {
                    i += p.chars().count();
                    out.push(Tok::Punct(p));
                }
                None if "+-*/%<>=!&|^~?:.,()[]".contains(c) => {
                    i += 1;
                    out.push(Tok::Punct(c.to_string()));
                }
                None => {
                    i += 1;
                    out.push(Tok::Bad);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locals() -> HashMap<String, String> {
        [("x", "int"), ("name", "string"), ("subtotal", "decimal"), ("order", "Shop.Core.Order"), ("lines", "Shop.Core.OrderLine[]"), ("count", "int?"), ("ok", "bool")]
            .into_iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect()
    }

    fn fx(e: &str) -> (bool, bool) {
        let f = side_effects(e, &locals());
        (f.implicit_calls, f.method_calls)
    }

    #[test]
    fn locals_literals_and_primitive_operators_are_harmless() {
        for e in ["x", "order", "this", "subtotal", "lines", "42", "1.5m", "\"a\\\"b\"", "@\"c:\\x\"\"y\"", "'\\''", "x * 10 + 1", "!ok && x >= 2", "name == \"Ada\"", "lines[x]", "lines[0]", "(x + 1) * 2", "order is null", "order as object", "x is int", "ok ? x : -1", "-x", ""] {
            assert_eq!(fx(e), (false, false), "{e}");
        }
    }

    #[test]
    fn member_access_unknown_names_indexers_and_user_operators_are_implicit_calls() {
        // A member may be a property; an unknown name may be a property of `this` or static.
        for e in ["order.Total", "order?.Customer", "this.Name", "Total", "Math.PI", "order.Lines[0]", "name[0]", "subtotal + 1", "-subtotal", "count + 1", "order == order", "order == null", "(int)subtotal", "(decimal)x", "lines[0].Price", "x::y"] {
            assert_eq!(fx(e), (true, false), "{e}");
        }
    }

    #[test]
    fn calls_creation_lambdas_and_assignments_are_method_calls() {
        assert_eq!(fx("Compute()"), (false, true));
        assert!(fx("order.Total()").1);
        assert_eq!(fx("order.Lines.Sum(l => l.Price)"), (true, true));
        assert!(fx("new Order()").1);
        assert_eq!(fx("x = 5"), (false, true));
        assert_eq!(fx("x++"), (false, true));
        assert_eq!(fx("x += 1"), (false, true));
        assert!(fx("Get<int>(x)").1);
        assert!(fx("typeof(int)").1);
    }

    #[test]
    fn what_is_not_understood_counts_as_both() {
        for e in ["$\"{order.Total}\"", "\"open", "{ x }", "x; y", "#x", "`"] {
            assert_eq!(fx(e), (true, true), "{e}");
        }
    }

    #[test]
    fn the_policy_permits_what_it_allows() {
        let none = EvalPolicy { implicit_calls: false, method_calls: false };
        let props = EvalPolicy { implicit_calls: true, method_calls: false };
        let member = SideEffects { implicit_calls: true, method_calls: false };
        let call = SideEffects { implicit_calls: false, method_calls: true };
        assert!(EvalPolicy::default().unrestricted() && EvalPolicy::default().permits(member) && EvalPolicy::default().permits(call));
        assert!(none.permits(SideEffects::default()) && !none.permits(member) && !none.permits(call));
        assert!(props.permits(member) && !props.permits(call) && !props.unrestricted());
    }
}
