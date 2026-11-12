//! Local C# interface, override, and delegation candidates for the Generate picker.

use crate::generate::{Kind, Member, TargetMember};
use rc_syntax::tree_sitter::{Node, Tree};

/// Builds picker candidates from a declaration resolved by Roslyn in another source file.
pub fn collect_resolved_source(
    source: &str,
    reference: &str,
    receiver: Option<&str>,
    existing: &[String],
) -> Vec<TargetMember> {
    let rope = rc_text::Rope::from_str(source);
    let Some(tree) = rc_syntax::parse_now(rc_syntax::Lang::CSharp, &rope, None) else {
        return Vec::new();
    };
    let mut types = Vec::new();
    walk_types(tree.root_node(), source, &mut types);
    let key = type_key(reference);
    let Some((declaration_kind, declaration)) = types.into_iter().find(|(_, node)| {
        node.child_by_field_name("name").is_some_and(|name| text(name, source) == key)
    }) else {
        return Vec::new();
    };
    let kind = if receiver.is_some() {
        Kind::Delegate
    } else if declaration_kind == "interface_declaration" {
        Kind::Implement
    } else {
        Kind::Override
    };
    let mut targets = Vec::new();
    add_type_members(&mut targets, source, declaration, kind, reference, receiver, existing);
    targets
}

pub fn collect(
    tree: &Tree,
    source: &str,
    current: Node<'_>,
    members: &[Member],
    existing: &[String],
) -> Vec<TargetMember> {
    let mut types = Vec::new();
    walk_types(tree.root_node(), source, &mut types);
    let mut result = Vec::new();
    let mut bases = Vec::new();
    if let Some(base_list) = named(current, "base_list") {
        let mut cursor = base_list.walk();
        for base in base_list.named_children(&mut cursor) {
            if base.kind() == "type"
                || base.kind().ends_with("_type")
                || base.kind() == "identifier"
                || base.kind() == "generic_name"
            {
                bases.push(text(base, source).to_string());
            }
        }
    }
    for base in bases {
        let key = type_key(&base);
        let Some((kind, declaration)) = types.iter().find(|(_, node)| {
            node.child_by_field_name("name")
                .is_some_and(|n| text(n, source) == key)
        }) else {
            continue;
        };
        let target_kind = if *kind == "interface_declaration" {
            Kind::Implement
        } else {
            Kind::Override
        };
        add_type_members(
            &mut result,
            source,
            *declaration,
            target_kind,
            &base,
            None,
            existing,
        );
    }
    for member in members {
        let key = type_key(&member.ty);
        let Some((_, declaration)) = types.iter().find(|(_, node)| {
            node.child_by_field_name("name")
                .is_some_and(|n| text(n, source) == key)
        }) else {
            continue;
        };
        add_type_members(
            &mut result,
            source,
            *declaration,
            Kind::Delegate,
            &member.ty,
            Some(&member.name),
            existing,
        );
    }
    result.sort_by(|a, b| {
        a.kind_label()
            .cmp(&b.kind_label())
            .then(a.label.cmp(&b.label))
    });
    result.dedup_by(|a, b| a.kind == b.kind && a.label == b.label);
    result
}

trait Label {
    fn kind_label(&self) -> u8;
}

impl Label for TargetMember {
    fn kind_label(&self) -> u8 {
        match self.kind {
            Kind::Implement => 0,
            Kind::Override => 1,
            Kind::Delegate => 2,
            _ => 3,
        }
    }
}

fn walk_types<'a>(node: Node<'a>, source: &str, out: &mut Vec<(&'static str, Node<'a>)>) {
    if matches!(
        node.kind(),
        "class_declaration" | "interface_declaration" | "struct_declaration"
    ) && node
        .child_by_field_name("name")
        .is_some_and(|name| !text(name, source).is_empty())
    {
        out.push((node.kind(), node));
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_types(child, source, out);
    }
}

fn add_type_members(
    out: &mut Vec<TargetMember>,
    source: &str,
    declaration: Node<'_>,
    kind: Kind,
    type_name: &str,
    receiver: Option<&str>,
    existing: &[String],
) {
    let substitutions = type_substitutions(type_name, declaration, source);
    let Some(body) = declaration.child_by_field_name("body") else {
        return;
    };
    let mut cursor = body.walk();
    for member in body.named_children(&mut cursor) {
        if matches!(
            member.kind(),
            "event_declaration" | "event_field_declaration" | "indexer_declaration"
        ) {
            add_special_member(
                out,
                source,
                member,
                kind,
                type_name,
                receiver,
                existing,
                &substitutions,
                declaration.kind() == "interface_declaration",
            );
            continue;
        }
        if !matches!(member.kind(), "method_declaration" | "property_declaration") {
            continue;
        }
        let Some(name_node) = member.child_by_field_name("name") else {
            continue;
        };
        let name = text(name_node, source);
        if existing.iter().any(|candidate| candidate == name) {
            continue;
        }
        let declaration_text = text(member, source);
        if declaration_text
            .split_whitespace()
            .any(|word| word == "static")
        {
            continue;
        }
        if kind == Kind::Override
            && !["virtual", "abstract", "override"].iter().any(|modifier| {
                declaration_text
                    .split_whitespace()
                    .any(|word| word == *modifier)
            })
        {
            continue;
        }
        if kind == Kind::Delegate
            && !declaration_text
                .split_whitespace()
                .any(|word| word == "public")
            && declaration.kind() != "interface_declaration"
        {
            continue;
        }
        let abstract_member = declaration_text
            .split_whitespace()
            .any(|word| word == "abstract");
        let access = if kind == Kind::Override
            && declaration_text
                .split_whitespace()
                .any(|word| word == "protected")
        {
            "protected"
        } else {
            "public"
        };
        let modifier = if kind == Kind::Override {
            " override"
        } else {
            ""
        };
        let Some(ty_node) = member.child_by_field_name(if member.kind() == "method_declaration" {
            "returns"
        } else {
            "type"
        }) else {
            continue;
        };
        let ty = substitute(text(ty_node, source), &substitutions);
        let code = if member.kind() == "method_declaration" {
            let Some(params_node) = member.child_by_field_name("parameters") else {
                continue;
            };
            let params = substitute(text(params_node, source), &substitutions);
            let args = arguments(params_node, source);
            let generic = member
                .child_by_field_name("type_parameters")
                .map(|n| text(n, source))
                .unwrap_or_default();
            let call = if let Some(receiver) = receiver {
                format!("{receiver}.{name}({args})")
            } else {
                format!("base.{name}({args})")
            };
            let body = if kind == Kind::Implement || abstract_member {
                "throw new global::System.NotImplementedException();".to_string()
            } else if ty == "void" {
                format!("{call};")
            } else {
                format!("return {call};")
            };
            format!("{access}{modifier} {ty} {name}{generic}{params}\n{{\n    {body}\n}}")
        } else {
            let accessors = member.child_by_field_name("accessors");
            let access_text = accessors.map(|n| text(n, source)).unwrap_or_default();
            let getter = access_text.contains("get") || accessors.is_none();
            let setter = access_text.contains("set");
            let receiver = receiver.unwrap_or("base");
            let get_body = if kind == Kind::Implement || abstract_member {
                "throw new global::System.NotImplementedException()".to_string()
            } else {
                format!("{receiver}.{name}")
            };
            let set_body = if kind == Kind::Implement || abstract_member {
                "throw new global::System.NotImplementedException()".to_string()
            } else {
                format!("{receiver}.{name} = value")
            };
            let mut lines = Vec::new();
            if getter {
                lines.push(format!("    get => {get_body};"));
            }
            if setter {
                lines.push(format!("    set => {set_body};"));
            }
            if lines.is_empty() {
                continue;
            }
            format!(
                "{access}{modifier} {ty} {name}\n{{\n{}\n}}",
                lines.join("\n")
            )
        };
        let label = if let Some(receiver) = receiver {
            format!("{receiver} → {type_name}.{name}: {ty}")
        } else {
            format!("{type_name}.{name}: {ty}")
        };
        out.push(TargetMember {
            kind,
            label,
            code,
            selected: true,
        });
    }
}

fn add_special_member(
    out: &mut Vec<TargetMember>,
    source: &str,
    member: Node<'_>,
    kind: Kind,
    type_name: &str,
    receiver: Option<&str>,
    existing: &[String],
    substitutions: &[(String, String)],
    interface: bool,
) {
    let raw = text(member, source);
    let words = raw.split_whitespace().collect::<Vec<_>>();
    if words.contains(&"static")
        || kind == Kind::Override
            && !["virtual", "abstract", "override"]
                .iter()
                .any(|m| words.contains(m))
        || kind == Kind::Delegate && !interface && !words.contains(&"public")
    {
        return;
    }
    let abstract_member = words.contains(&"abstract");
    let access = if kind == Kind::Override && words.contains(&"protected") {
        "protected"
    } else {
        "public"
    };
    let modifier = if kind == Kind::Override {
        " override"
    } else {
        ""
    };
    let source_ty = if member.kind() == "event_field_declaration" {
        named(member, "variable_declaration").and_then(|d| d.named_child(0))
    } else {
        member.child_by_field_name("type")
    };
    let Some(source_ty) = source_ty else { return };
    let ty = substitute(text(source_ty, source), substitutions);
    let names = if member.kind() == "event_field_declaration" {
        let Some(decl) = named(member, "variable_declaration") else {
            return;
        };
        let mut cursor = decl.walk();
        decl.named_children(&mut cursor)
            .filter(|n| n.kind() == "variable_declarator")
            .filter_map(|n| n.child_by_field_name("name"))
            .map(|n| text(n, source).to_string())
            .collect::<Vec<_>>()
    } else if member.kind() == "indexer_declaration" {
        vec!["this".to_string()]
    } else {
        member
            .child_by_field_name("name")
            .map(|n| vec![text(n, source).to_string()])
            .unwrap_or_default()
    };
    for name in names {
        if existing.iter().any(|candidate| candidate == &name) {
            continue;
        }
        let code = if member.kind() == "indexer_declaration" {
            let Some(parameters) = member.child_by_field_name("parameters") else {
                continue;
            };
            let params = substitute(text(parameters, source), substitutions);
            let args = arguments(parameters, source);
            let target = format!("{}[{args}]", receiver.unwrap_or("base"));
            let access_text = member
                .child_by_field_name("accessors")
                .map(|n| text(n, source))
                .unwrap_or_default();
            let getter = access_text.contains("get") || access_text.is_empty();
            let setter = access_text.contains("set");
            let mut lines = Vec::new();
            if getter {
                lines.push(format!(
                    "    get => {};",
                    if kind == Kind::Implement || abstract_member {
                        "throw new global::System.NotImplementedException()".to_string()
                    } else {
                        target.clone()
                    }
                ));
            }
            if setter {
                lines.push(format!(
                    "    set => {};",
                    if kind == Kind::Implement || abstract_member {
                        "throw new global::System.NotImplementedException()".to_string()
                    } else {
                        format!("{target} = value")
                    }
                ));
            }
            if lines.is_empty() {
                continue;
            }
            format!(
                "{access}{modifier} {ty} this{params}\n{{\n{}\n}}",
                lines.join("\n")
            )
        } else {
            let add = if kind == Kind::Implement || abstract_member {
                "throw new global::System.NotImplementedException()".to_string()
            } else {
                format!("{}.{} += value", receiver.unwrap_or("base"), name)
            };
            let remove = if kind == Kind::Implement || abstract_member {
                "throw new global::System.NotImplementedException()".to_string()
            } else {
                format!("{}.{} -= value", receiver.unwrap_or("base"), name)
            };
            format!(
                "{access}{modifier} event {ty} {name}\n{{\n    add => {add};\n    remove => {remove};\n}}"
            )
        };
        let label = if let Some(receiver) = receiver {
            format!("{receiver} → {type_name}.{name}: {ty}")
        } else {
            format!("{type_name}.{name}: {ty}")
        };
        out.push(TargetMember {
            kind,
            label,
            code,
            selected: true,
        });
    }
}

fn arguments(params: Node<'_>, source: &str) -> String {
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .filter(|n| n.kind() == "parameter")
        .filter_map(|parameter| {
            let name = text(parameter.child_by_field_name("name")?, source);
            let raw = text(parameter, source);
            let modifier = if raw.starts_with("ref ") {
                "ref "
            } else if raw.starts_with("out ") {
                "out "
            } else if raw.starts_with("in ") {
                "in "
            } else {
                ""
            };
            Some(format!("{modifier}{name}"))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn type_key(ty: &str) -> String {
    ty.trim()
        .split('<')
        .next()
        .unwrap_or_default()
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches('?')
        .to_string()
}

fn type_substitutions(
    reference: &str,
    declaration: Node<'_>,
    source: &str,
) -> Vec<(String, String)> {
    let Some(start) = reference.find('<') else {
        return Vec::new();
    };
    let Some(end) = reference.rfind('>') else {
        return Vec::new();
    };
    let Some(parameters) = named(declaration, "type_parameter_list") else {
        return Vec::new();
    };
    let mut cursor = parameters.walk();
    let names = parameters
        .named_children(&mut cursor)
        .filter(|node| node.kind() == "type_parameter")
        .map(|node| text(node, source).to_string())
        .collect::<Vec<_>>();
    let values = split_generic_arguments(&reference[start + 1..end]);
    if names.len() != values.len() {
        return Vec::new();
    }
    names.into_iter().zip(values).collect()
}

fn split_generic_arguments(input: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, ch) in input.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                result.push(input[start..index].trim().to_string());
                start = index + 1;
            }
            _ => {}
        }
    }
    result.push(input[start..].trim().to_string());
    result
}

fn substitute(input: &str, mappings: &[(String, String)]) -> String {
    let mut result = String::new();
    let mut start = 0;
    for (index, ch) in input.char_indices() {
        if ch.is_alphanumeric() || ch == '_' {
            continue;
        }
        if start < index {
            let word = &input[start..index];
            result.push_str(
                mappings
                    .iter()
                    .find(|(name, _)| name == word)
                    .map_or(word, |(_, value)| value),
            );
        }
        result.push(ch);
        start = index + ch.len_utf8();
    }
    if start < input.len() {
        let word = &input[start..];
        result.push_str(
            mappings
                .iter()
                .find(|(name, _)| name == word)
                .map_or(word, |(_, value)| value),
        );
    }
    result
}

fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

fn named<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

#[cfg(test)]
mod resolved_tests {
    use super::*;

    #[test]
    fn resolves_members_from_another_source_file() {
        let source = "namespace Contracts; public interface IRepository<T> { T Find(T value); string Name { get; } }";
        let targets = collect_resolved_source(source, "IRepository<string>", None, &[]);
        assert!(targets.iter().any(|target| target.kind == Kind::Implement && target.code.contains("string Find(string value)")));
        assert!(targets.iter().any(|target| target.code.contains("string Name")));

        let service = "namespace Services; public class Formatter { public string Format(int value) => value.ToString(); }";
        let targets = collect_resolved_source(service, "Formatter", Some("_formatter"), &[]);
        assert!(targets.iter().any(|target| target.kind == Kind::Delegate && target.code.contains("_formatter.Format(value)")));
    }
}
