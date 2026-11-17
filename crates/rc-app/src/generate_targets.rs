//! Interface, override and delegating members for the Generate picker, read with tree-sitter
//! from the declarations of the type's base types and of its fields' and properties' types.
//!
//! Generated code uses one `\t` per nested level (see [`crate::generate::Insertion`]).

use crate::generate::{Kind, Member, Signature, Style, TargetMember, modifiers, named, parameter_types, text};
use rc_syntax::tree_sitter::{Node, Tree};

/// What the type already has, so members it declares are not offered again.
#[derive(Clone, Debug, Default)]
pub struct Known {
    pub existing: Vec<String>,
    pub methods: Vec<Signature>,
    pub style: Style,
}

impl Known {
    fn has(&self, name: &str, params: Option<&[String]>) -> bool {
        match params {
            Some(params) => self.methods.iter().any(|s| s.name == name && s.params == params),
            None => self.existing.iter().any(|n| n == name),
        }
    }
}

/// Builds picker candidates from a declaration resolved by Roslyn in another source file.
pub fn collect_resolved_source(source: &str, reference: &str, receiver: Option<&str>, known: &Known) -> Vec<TargetMember> {
    let rope = rc_text::Rope::from_str(source);
    let Some(tree) = rc_syntax::parse_now(rc_syntax::Lang::CSharp, &rope, None) else { return Vec::new() };
    let mut types = Vec::new();
    walk_types(tree.root_node(), source, &mut types);
    let Some(declaration) = find_type(&types, source, reference) else { return Vec::new() };
    let kind = if receiver.is_some() {
        Kind::Delegate
    } else if declaration.kind() == "interface_declaration" {
        Kind::Implement
    } else {
        Kind::Override
    };
    let mut targets = Vec::new();
    add_type_members(&mut targets, source, declaration, kind, reference, receiver, known);
    targets
}

/// Candidates from the types declared in the same file as `current`.
pub fn collect(tree: &Tree, source: &str, current: Node<'_>, members: &[Member], known: &Known) -> Vec<TargetMember> {
    let mut types = Vec::new();
    walk_types(tree.root_node(), source, &mut types);
    let mut result = Vec::new();
    for base in base_types(current, source) {
        let Some(declaration) = find_type(&types, source, &base) else { continue };
        let kind = if declaration.kind() == "interface_declaration" { Kind::Implement } else { Kind::Override };
        add_type_members(&mut result, source, declaration, kind, &base, None, known);
    }
    for member in members {
        let Some(declaration) = find_type(&types, source, &member.ty) else { continue };
        add_type_members(&mut result, source, declaration, Kind::Delegate, &member.ty, Some(&member.name), known);
    }
    dedup(&mut result);
    result
}

/// Keeps the first of two candidates that would generate the same member.
pub fn dedup(targets: &mut Vec<TargetMember>) {
    let mut seen = std::collections::HashSet::new();
    targets.retain(|t| seen.insert((t.kind, t.name.clone(), t.params.clone())));
}

/// The types in the base list of a declaration, as written (`Base`, `IRepository<string>`).
pub fn base_types(declaration: Node<'_>, source: &str) -> Vec<String> {
    let Some(list) = named(declaration, "base_list") else { return Vec::new() };
    let mut cursor = list.walk();
    list.named_children(&mut cursor)
        .filter_map(|base| match base.kind() {
            "argument_list" => None,
            // `class Derived(int x) : Base(x)`
            "primary_constructor_base_type" => base.child_by_field_name("type").map(|t| text(t, source).to_string()),
            _ => Some(text(base, source).to_string()),
        })
        .collect()
}

pub(crate) fn walk_types<'a>(node: Node<'a>, source: &str, out: &mut Vec<Node<'a>>) {
    if matches!(node.kind(), "class_declaration" | "interface_declaration" | "struct_declaration" | "record_declaration")
        && node.child_by_field_name("name").is_some_and(|name| !text(name, source).is_empty())
    {
        out.push(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_types(child, source, out);
    }
}

/// The declaration a type reference names: same name and number of type arguments.
pub(crate) fn find_type<'a>(types: &[Node<'a>], source: &str, reference: &str) -> Option<Node<'a>> {
    let key = type_key(reference);
    let arity = generic_arguments(reference).len();
    types.iter().copied().find(|node| {
        let params = named(*node, "type_parameter_list").map_or(0, |list| {
            let mut cursor = list.walk();
            list.named_children(&mut cursor).filter(|n| n.kind() == "type_parameter").count()
        });
        params == arity && node.child_by_field_name("name").is_some_and(|n| text(n, source) == key)
    })
}

/// The accessors of a property, indexer or event: name and modifiers (`private set`).
fn accessor_list(node: Node<'_>, source: &str) -> Vec<(String, Vec<String>)> {
    let Some(list) = node.child_by_field_name("accessors") else { return Vec::new() };
    let mut cursor = list.walk();
    list.named_children(&mut cursor)
        .filter(|a| a.kind() == "accessor_declaration")
        .filter_map(|a| Some((text(a.child_by_field_name("name")?, source).to_string(), modifiers(a, source))))
        .collect()
}

/// Whether an interface member has a default implementation (nothing to implement).
fn has_body(member: Node<'_>) -> bool {
    if member.child_by_field_name("body").is_some() || member.child_by_field_name("value").is_some_and(|v| v.kind() == "arrow_expression_clause") {
        return true;
    }
    let Some(list) = member.child_by_field_name("accessors") else { return false };
    let mut cursor = list.walk();
    list.named_children(&mut cursor).any(|a| a.child_by_field_name("body").is_some())
}

pub(crate) fn add_type_members(out: &mut Vec<TargetMember>, source: &str, declaration: Node<'_>, kind: Kind, type_name: &str, receiver: Option<&str>, known: &Known) {
    let interface = declaration.kind() == "interface_declaration";
    let substitutions = type_substitutions(type_name, declaration, source);
    let Some(body) = declaration.child_by_field_name("body") else { return };
    let origin = match receiver {
        Some(receiver) => format!("{receiver}: {type_name}"),
        None => type_name.to_string(),
    };
    let mut cursor = body.walk();
    for member in body.named_children(&mut cursor) {
        if !matches!(member.kind(), "method_declaration" | "property_declaration" | "indexer_declaration" | "event_declaration" | "event_field_declaration") {
            continue;
        }
        let mods = modifiers(member, source);
        let has = |m: &str| mods.iter().any(|x| x == m);
        if has("static") || has("private") || named(member, "explicit_interface_specifier").is_some() {
            continue;
        }
        let candidate = match kind {
            Kind::Implement => interface && !has_body(member),
            Kind::Override => (has("virtual") || has("abstract") || has("override")) && !has("sealed"),
            _ => interface || has("public"),
        };
        if !candidate {
            continue;
        }
        let spec = Spec {
            kind,
            receiver,
            // An abstract member has no base to call.
            stub: kind == Kind::Implement || (kind == Kind::Override && has("abstract")),
            access: if kind == Kind::Override { accessibility(&mods) } else { "public".into() },
            modifier: if kind == Kind::Override { " override" } else { "" },
            substitutions: &substitutions,
            known,
        };
        for mut target in spec.members(member, source) {
            target.source = origin.clone();
            out.push(target);
        }
    }
}

/// How to write the members of one declaration.
struct Spec<'a> {
    kind: Kind,
    receiver: Option<&'a str>,
    /// Bodies throw `NotImplementedException` (interfaces and abstract members).
    stub: bool,
    access: String,
    modifier: &'static str,
    substitutions: &'a [(String, String)],
    known: &'a Known,
}

impl Spec<'_> {
    fn sub(&self, input: &str) -> String {
        substitute(input, self.substitutions)
    }

    fn throw(&self) -> String {
        format!("throw new {}()", self.known.style.system("NotImplementedException"))
    }

    /// What a member forwards to: `_inner.Name` or `base.Name`.
    fn target(&self, name: &str) -> String {
        format!("{}.{name}", self.receiver.unwrap_or("base"))
    }

    fn member(&self, name: String, params: Option<Vec<String>>, label: String, code: String) -> Option<TargetMember> {
        if self.known.has(&name, params.as_deref()) {
            return None;
        }
        Some(TargetMember { kind: self.kind, label, code, selected: true, name, params, source: String::new() })
    }

    fn members(&self, member: Node<'_>, source: &str) -> Vec<TargetMember> {
        match member.kind() {
            "method_declaration" => self.method(member, source).into_iter().collect(),
            "property_declaration" => self.property(member, source).into_iter().collect(),
            "indexer_declaration" => self.indexer(member, source).into_iter().collect(),
            _ => self.events(member, source),
        }
    }

    fn method(&self, member: Node<'_>, source: &str) -> Option<TargetMember> {
        let name = text(member.child_by_field_name("name")?, source).to_string();
        let ret = self.sub(text(member.child_by_field_name("returns")?, source));
        let params_node = member.child_by_field_name("parameters")?;
        let params = self.sub(text(params_node, source));
        let types: Vec<String> = parameter_types(params_node, source).iter().map(|t| self.sub(t)).collect();
        let type_params = member.child_by_field_name("type_parameters");
        let generic = type_params.map(|n| text(n, source)).unwrap_or_default();
        let type_args = type_params.map(|list| {
            let mut cursor = list.walk();
            let names: Vec<&str> = list.named_children(&mut cursor).filter(|n| n.kind() == "type_parameter").filter_map(|n| n.child_by_field_name("name")).map(|n| text(n, source)).collect();
            format!("<{}>", names.join(", "))
        });
        // An implementation repeats the constraints; an override inherits them.
        let constraints: String = if self.kind == Kind::Override {
            String::new()
        } else {
            let mut cursor = member.walk();
            member.named_children(&mut cursor).filter(|n| n.kind() == "type_parameter_constraints_clause").map(|n| format!(" {}", self.sub(text(n, source)))).collect()
        };
        let body = if self.stub {
            format!("{};", self.throw())
        } else {
            let call = format!("{}{}({})", self.target(&name), type_args.unwrap_or_default(), arguments(params_node, source));
            if ret == "void" { format!("{call};") } else { format!("return {call};") }
        };
        let code = format!("{}{} {ret} {name}{generic}{params}{constraints}\n{{\n\t{body}\n}}", self.access, self.modifier);
        let label = format!("{name}{generic}{params}: {ret}");
        self.member(name, Some(types), label, code)
    }

    /// Accessors to write: the declaration's, minus the ones this type cannot reach.
    fn usable_accessors(&self, member: Node<'_>, source: &str) -> Vec<String> {
        let list = accessor_list(member, source);
        if list.is_empty() {
            // `int X => 1;`: a getter.
            return vec!["get".into()];
        }
        list.into_iter()
            .filter(|(_, mods)| !mods.iter().any(|m| m == "private"))
            // An init-only property cannot be set from another object's setter.
            .filter(|(name, _)| !(self.kind == Kind::Delegate && name == "init"))
            .map(|(name, _)| name)
            .collect()
    }

    fn property(&self, member: Node<'_>, source: &str) -> Option<TargetMember> {
        let name = text(member.child_by_field_name("name")?, source).to_string();
        let ty = self.sub(text(member.child_by_field_name("type")?, source));
        let accessors = self.usable_accessors(member, source);
        // Interface properties get stubs too: an auto-property of a reference type would warn
        // (CS8618) in a nullable project.
        let head = format!("{}{} {ty} {name}", self.access, self.modifier);
        let code = self.accessor_block(&head, &accessors, &self.target(&name))?;
        self.member(name.clone(), None, format!("{name}: {ty}"), code)
    }

    fn indexer(&self, member: Node<'_>, source: &str) -> Option<TargetMember> {
        let ty = self.sub(text(member.child_by_field_name("type")?, source));
        let params_node = member.child_by_field_name("parameters")?;
        let params = self.sub(text(params_node, source));
        let types: Vec<String> = parameter_types(params_node, source).iter().map(|t| self.sub(t)).collect();
        let accessors = self.usable_accessors(member, source);
        let target = format!("{}[{}]", self.receiver.unwrap_or("base"), arguments(params_node, source));
        let code = self.accessor_block(&format!("{}{} {ty} this{params}", self.access, self.modifier), &accessors, &target)?;
        self.member("this".into(), Some(types), format!("this{params}: {ty}"), code)
    }

    /// `head` with `get => target;`-style accessors; one getter becomes `head => target;`.
    fn accessor_block(&self, head: &str, accessors: &[String], target: &str) -> Option<String> {
        if accessors.is_empty() {
            return None;
        }
        if accessors == ["get"] {
            return Some(format!("{head} => {};", if self.stub { self.throw() } else { target.to_string() }));
        }
        let lines: Vec<String> = accessors
            .iter()
            .map(|a| match (self.stub, a.as_str()) {
                (true, _) => format!("\t{a} => {};", self.throw()),
                (false, "get") => format!("\tget => {target};"),
                (false, _) => format!("\t{a} => {target} = value;"),
            })
            .collect();
        Some(format!("{head}\n{{\n{}\n}}", lines.join("\n")))
    }

    fn events(&self, member: Node<'_>, source: &str) -> Vec<TargetMember> {
        let (ty, names) = if member.kind() == "event_field_declaration" {
            let Some(decl) = named(member, "variable_declaration") else { return Vec::new() };
            let Some(ty) = decl.child_by_field_name("type") else { return Vec::new() };
            let mut cursor = decl.walk();
            let names = decl.named_children(&mut cursor).filter(|n| n.kind() == "variable_declarator").filter_map(|n| n.child_by_field_name("name")).map(|n| text(n, source).to_string()).collect();
            (ty, names)
        } else {
            let (Some(ty), Some(name)) = (member.child_by_field_name("type"), member.child_by_field_name("name")) else { return Vec::new() };
            (ty, vec![text(name, source).to_string()])
        };
        let ty = self.sub(text(ty, source));
        let head = format!("{}{} event", self.access, self.modifier);
        names
            .into_iter()
            .filter_map(|name| {
                let code = if self.stub {
                    // A field-like event implements (or overrides) the declaration.
                    let nullable = if self.known.style.nullable && !ty.ends_with('?') { "?" } else { "" };
                    format!("{head} {ty}{nullable} {name};")
                } else {
                    let target = self.target(&name);
                    format!("{head} {ty} {name}\n{{\n\tadd => {target} += value;\n\tremove => {target} -= value;\n}}")
                };
                self.member(name.clone(), None, format!("{name}: {ty}"), code)
            })
            .collect()
    }
}

/// The accessibility to override with: the same as the overridden member.
fn accessibility(mods: &[String]) -> String {
    let access: Vec<&str> = mods.iter().map(String::as_str).filter(|m| matches!(*m, "public" | "protected" | "internal" | "private")).collect();
    if access.is_empty() { "public".into() } else { access.join(" ") }
}

/// The arguments that pass a parameter list on: `ref a, out b, c`.
fn arguments(params: Node<'_>, source: &str) -> String {
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .filter(|n| n.kind() == "parameter")
        .filter_map(|parameter| {
            let name = text(parameter.child_by_field_name("name")?, source);
            let modifier = modifiers(parameter, source).into_iter().find(|m| matches!(m.as_str(), "ref" | "out" | "in"));
            Some(match modifier {
                Some(m) => format!("{m} {name}"),
                None => name.to_string(),
            })
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `global::Contracts.IRepository<string>?` → `IRepository`.
pub(crate) fn type_key(ty: &str) -> String {
    let bare = ty.trim().split('<').next().unwrap_or_default();
    bare.rsplit(['.', ':']).next().unwrap_or_default().trim_end_matches('?').to_string()
}

/// The type arguments of a reference: `Dictionary<string, List<int>>` → `string`, `List<int>`.
fn generic_arguments(reference: &str) -> Vec<String> {
    let (Some(start), Some(end)) = (reference.find('<'), reference.rfind('>')) else { return Vec::new() };
    if end <= start {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut depth = 0usize;
    let inner = &reference[start + 1..end];
    let mut from = 0;
    for (index, ch) in inner.char_indices() {
        match ch {
            '<' | '(' => depth += 1,
            '>' | ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                result.push(inner[from..index].trim().to_string());
                from = index + 1;
            }
            _ => {}
        }
    }
    result.push(inner[from..].trim().to_string());
    result
}

/// Type parameter → argument pairs: `IRepository<T>` used as `IRepository<string>`.
fn type_substitutions(reference: &str, declaration: Node<'_>, source: &str) -> Vec<(String, String)> {
    let Some(parameters) = named(declaration, "type_parameter_list") else { return Vec::new() };
    let mut cursor = parameters.walk();
    // The name only: `out T` and attributes are not part of it.
    let names: Vec<String> = parameters.named_children(&mut cursor).filter(|node| node.kind() == "type_parameter").filter_map(|n| n.child_by_field_name("name")).map(|n| text(n, source).to_string()).collect();
    let values = generic_arguments(reference);
    if names.len() != values.len() {
        return Vec::new();
    }
    names.into_iter().zip(values).collect()
}

/// Replaces whole identifiers of `input` by their mapping.
fn substitute(input: &str, mappings: &[(String, String)]) -> String {
    if mappings.is_empty() {
        return input.to_string();
    }
    let mut result = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, result: &mut String| {
        if !word.is_empty() {
            result.push_str(mappings.iter().find(|(name, _)| name == word).map_or(word.as_str(), |(_, value)| value));
            word.clear();
        }
    };
    for ch in input.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            word.push(ch);
        } else {
            flush(&mut word, &mut result);
            result.push(ch);
        }
    }
    flush(&mut word, &mut result);
    result
}

#[cfg(test)]
mod tests {
    use crate::generate::{Dialog, Kind, Style};

    fn parse(source: &str) -> rc_syntax::tree_sitter::Tree {
        rc_syntax::parse_now(rc_syntax::Lang::CSharp, &rc_text::Rope::from_str(source), None).unwrap()
    }

    fn generate(source: &str, at: &str, kind: Kind, style: Style) -> String {
        let tree = parse(source);
        let mut d = Dialog::from_source(source, &tree, source.find(at).unwrap(), 1, 0, "\t", style).unwrap();
        d.kind = kind;
        let (range, text) = d.edit().unwrap();
        let out = format!("{}{}{}", &source[..range.start], text, &source[range.end..]);
        assert!(!parse(&out).root_node().has_error(), "{out}");
        out
    }

    #[test]
    fn interfaces_become_stubs_and_field_like_events() {
        let source = "interface IShape<out T> where T : class\n{\n\tT Name { get; init; }\n\tevent System.EventHandler Changed;\n\tvoid Move<U>(ref U by, out int moved) where U : struct;\n\tint Area() => 0;\n}\nclass Square : IShape<string>\n{\n\tpublic void Other() { }\n}\n";
        let style = Style { nullable: true, system: true, collections: true };
        let out = generate(source, "Other", Kind::Implement, style);
        assert!(out.contains("\tpublic string Name\n\t{\n\t\tget => throw new NotImplementedException();\n\t\tinit => throw new NotImplementedException();\n\t}\n"), "{out}");
        assert!(out.contains("\tpublic event System.EventHandler? Changed;\n"), "{out}");
        assert!(out.contains("\tpublic void Move<U>(ref U by, out int moved) where U : struct\n\t{\n\t\tthrow new NotImplementedException();\n\t}\n"), "{out}");
        assert!(!out.contains("public int Area"), "default implementations are not missing: {out}");
    }

    #[test]
    fn overrides_keep_accessibility_and_skip_sealed_members() {
        let source = "abstract class Shape\n{\n\tprotected internal virtual void Draw(int x) { }\n\tpublic abstract double Area { get; }\n\tpublic sealed override string ToString() => \"\";\n\tprotected virtual T Copy<T>() where T : new() => new T();\n\tpublic virtual int this[int i] { get => i; set { } }\n\tpublic void Plain() { }\n\tprivate static void Helper() { /* virtual */ }\n}\nclass Circle : Shape\n{\n\tprivate double _r;\n}\n";
        let out = generate(source, "_r", Kind::Override, Style::default());
        assert!(out.contains("\tprotected internal override void Draw(int x)\n\t{\n\t\tbase.Draw(x);\n\t}\n"), "{out}");
        assert!(out.contains("\tpublic override double Area => throw new System.NotImplementedException();\n"), "{out}");
        assert!(out.contains("\tprotected override T Copy<T>()\n\t{\n\t\treturn base.Copy<T>();\n\t}\n"), "{out}");
        assert!(out.contains("\tpublic override int this[int i]\n\t{\n\t\tget => base[i];\n\t\tset => base[i] = value;\n\t}\n"), "{out}");
        assert!(!out.contains("public override string ToString") && !out.contains("override void Plain") && !out.contains("override void Helper"), "{out}");
    }

    #[test]
    fn delegates_forward_to_the_field() {
        let source = "class Service\n{\n\tpublic string Format<T>(T value, ref int count) => \"\";\n\tpublic int Count { get; private set; }\n\tpublic string Id { get; init; }\n\tpublic event System.Action Done;\n\tinternal void Hidden() { }\n}\nclass Facade\n{\n\tprivate readonly Service _service = new();\n}\n";
        let out = generate(source, "_service", Kind::Delegate, Style::default());
        assert!(out.contains("\tpublic string Format<T>(T value, ref int count)\n\t{\n\t\treturn _service.Format<T>(value, ref count);\n\t}\n"), "{out}");
        assert!(out.contains("\tpublic int Count => _service.Count;\n"), "{out}");
        assert!(out.contains("\tpublic string Id => _service.Id;\n"), "{out}");
        assert!(out.contains("\tpublic event System.Action Done\n\t{\n\t\tadd => _service.Done += value;\n\t\tremove => _service.Done -= value;\n\t}\n"), "{out}");
        assert!(!out.contains("_service.Hidden"), "{out}");
    }

    #[test]
    fn resolves_members_from_another_source_file() {
        use super::{Known, collect_resolved_source};
        let source = "namespace Contracts; public interface IRepository<T> { T Find(T value); string Name { get; } }";
        let targets = collect_resolved_source(source, "IRepository<string>", None, &Known::default());
        assert!(targets.iter().any(|target| target.kind == Kind::Implement && target.code.contains("string Find(string value)")));
        assert!(targets.iter().any(|target| target.code.contains("string Name")));
        let service = "namespace Services; public class Formatter { public string Format(int value) => value.ToString(); }";
        let targets = collect_resolved_source(service, "Formatter", Some("_formatter"), &Known::default());
        assert!(targets.iter().any(|target| target.kind == Kind::Delegate && target.code.contains("_formatter.Format(value)")));
    }

    #[test]
    fn members_the_type_has_are_not_offered() {
        let source = "interface IRepo\n{\n\tvoid Save(int id);\n\tvoid Save(string name);\n\tint Count { get; }\n}\nclass Repo : IRepo\n{\n\tpublic void Save(int id) { }\n\tint IRepo.Count => 0;\n}\n";
        let tree = parse(source);
        let d = Dialog::from_source(source, &tree, source.find("Save(int id) {").unwrap(), 1, 0, "\t", Style::default()).unwrap();
        let labels: Vec<_> = d.targets.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, ["Save(string name): void"], "the other overload and the explicit implementation exist");
    }
}
