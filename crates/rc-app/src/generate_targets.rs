//! Local C# interface, override, and delegation candidates for the Generate picker.

use crate::generate::{Kind, Member, TargetMember};
use rc_syntax::tree_sitter::{Node, Tree};

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
            {
                bases.push(type_key(text(base, source)));
            }
        }
    }
    for base in bases {
        let Some((kind, declaration)) = types.iter().find(|(_, node)| {
            node.child_by_field_name("name")
                .is_some_and(|n| text(n, source) == base)
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
            &key,
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
    let Some(body) = declaration.child_by_field_name("body") else {
        return;
    };
    let mut cursor = body.walk();
    for member in body.named_children(&mut cursor) {
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
        let ty = text(ty_node, source);
        let code = if member.kind() == "method_declaration" {
            let Some(params_node) = member.child_by_field_name("parameters") else {
                continue;
            };
            let params = text(params_node, source);
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

fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

fn named<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}
