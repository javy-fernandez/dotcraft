//! C# member generation from the active syntax tree.

use crate::docs::Doc;
use rc_syntax::tree_sitter::{Node, Tree};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Constructor,
    Properties,
    Equality,
    ToString,
    Formatting,
    Implement,
    Override,
    Delegate,
}

impl Kind {
    pub const ALL: [Self; 8] = [
        Self::Constructor,
        Self::Properties,
        Self::Equality,
        Self::ToString,
        Self::Formatting,
        Self::Implement,
        Self::Override,
        Self::Delegate,
    ];

    pub fn label(self) -> &'static str {
        use rc_kit::i18n::tr;
        match self {
            Self::Constructor => tr("Constructor"),
            Self::Properties => tr("Properties"),
            Self::Equality => tr("Equals and GetHashCode"),
            Self::ToString => tr("ToString"),
            Self::Formatting => tr("Formatting members"),
            Self::Implement => tr("Implement members"),
            Self::Override => tr("Override members"),
            Self::Delegate => tr("Delegate members"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Member {
    pub name: String,
    pub ty: String,
    pub field: bool,
    pub readonly: bool,
    pub selected: bool,
}

#[derive(Clone, Debug)]
pub struct TargetMember {
    pub kind: Kind,
    pub label: String,
    pub code: String,
    pub selected: bool,
}

#[derive(Clone, Debug)]
pub struct Dialog {
    pub doc_id: u64,
    pub version: u64,
    pub class_name: String,
    pub type_name: String,
    pub insert_byte: usize,
    pub indent: String,
    pub kind: Kind,
    pub members: Vec<Member>,
    pub targets: Vec<TargetMember>,
    pub existing: Vec<String>,
}

impl Dialog {
    pub fn from_doc(doc: &Doc) -> Option<Self> {
        if doc.editor.read_only || doc.path.extension()?.to_str()? != "cs" {
            return None;
        }
        if !doc.editor.syntax.is_current() {
            return None;
        }
        let tree = doc.editor.syntax.tree()?;
        let rope = doc.editor.buffer.rope();
        let source = rope.to_string();
        let byte = rope.char_to_byte(doc.editor.caret());
        Self::from_source(
            &source,
            tree,
            byte,
            doc.editor.id,
            doc.editor.buffer.version(),
        )
    }

    fn from_source(
        source: &str,
        tree: &Tree,
        byte: usize,
        doc_id: u64,
        version: u64,
    ) -> Option<Self> {
        let mut node = tree.root_node().descendant_for_byte_range(byte, byte)?;
        while !matches!(node.kind(), "class_declaration" | "struct_declaration") {
            node = node.parent()?;
        }
        let name = text(node.child_by_field_name("name")?, &source).to_string();
        let type_name = format!(
            "{}{}",
            name,
            named(node, "type_parameter_list")
                .map(|n| text(n, &source))
                .unwrap_or_default()
        );
        let body = node.child_by_field_name("body")?;
        let insert_byte = body.end_byte().checked_sub(1)?;
        let line_start = source[..node.start_byte()].rfind('\n').map_or(0, |i| i + 1);
        let indent = source[line_start..node.start_byte()]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect::<String>();
        let mut members = Vec::new();
        let mut existing = Vec::new();
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            match child.kind() {
                "field_declaration" => {
                    let Some(decl) = named(child, "variable_declaration") else {
                        continue;
                    };
                    let Some(ty) = decl.named_child(0) else {
                        continue;
                    };
                    let ty = text(ty, &source).to_string();
                    let mut c = decl.walk();
                    for variable in decl
                        .named_children(&mut c)
                        .filter(|n| n.kind() == "variable_declarator")
                    {
                        if let Some(name) = variable.child_by_field_name("name") {
                            members.push(Member {
                                name: text(name, &source).into(),
                                ty: ty.clone(),
                                field: true,
                                readonly: text(child, &source)
                                    .split_whitespace()
                                    .any(|word| word == "readonly"),
                                selected: true,
                            });
                        }
                    }
                }
                "property_declaration" => {
                    if let (Some(name), Some(ty)) = (
                        child.child_by_field_name("name"),
                        child.child_by_field_name("type"),
                    ) {
                        let name = text(name, &source).to_string();
                        existing.push(name.clone());
                        members.push(Member {
                            name,
                            ty: text(ty, &source).into(),
                            field: false,
                            readonly: false,
                            selected: true,
                        });
                    }
                }
                "method_declaration" | "constructor_declaration" => {
                    if let Some(name) = child.child_by_field_name("name") {
                        existing.push(text(name, &source).into());
                    }
                }
                _ => {}
            }
        }
        let targets = crate::generate_targets::collect(tree, source, node, &members, &existing);
        Some(Self {
            doc_id,
            version,
            class_name: name,
            type_name,
            insert_byte,
            indent,
            kind: Kind::Constructor,
            members,
            targets,
            existing,
        })
    }

    pub fn candidates(&self) -> impl Iterator<Item = (usize, &Member)> {
        self.members
            .iter()
            .enumerate()
            .filter(|(_, m)| match self.kind {
                Kind::Properties => {
                    m.field && !self.existing.iter().any(|n| n == &property_name(&m.name))
                }
                Kind::Constructor => {
                    m.field
                        || !self
                            .members
                            .iter()
                            .any(|f| f.field && parameter_name(&f.name) == parameter_name(&m.name))
                }
                _ => true,
            })
    }

    pub fn render(&self) -> Result<String, &'static str> {
        if matches!(self.kind, Kind::Implement | Kind::Override | Kind::Delegate) {
            let code = self
                .targets
                .iter()
                .filter(|m| m.kind == self.kind && m.selected)
                .map(|m| m.code.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            if code.is_empty() {
                return Err("Select at least one member");
            }
            return Ok(self.indent_code(&code));
        }
        let selected: Vec<_> = self
            .candidates()
            .filter(|(_, m)| m.selected)
            .map(|(_, m)| m)
            .collect();
        if selected.is_empty() {
            return Err("Select at least one member");
        }
        let name = &self.class_name;
        let body = match self.kind {
            Kind::Constructor => {
                if self.existing.iter().any(|n| n == name) {
                    return Err("A constructor already exists");
                }
                let params = selected
                    .iter()
                    .map(|m| format!("{} {}", m.ty, parameter_name(&m.name)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let assignments = selected
                    .iter()
                    .map(|m| format!("this.{} = {};", m.name, parameter_name(&m.name)))
                    .collect::<Vec<_>>();
                format!(
                    "public {name}({params})\n{{\n    {}\n}}",
                    assignments.join("\n    ")
                )
            }
            Kind::Properties => selected
                .iter()
                .map(|m| {
                    let property = property_name(&m.name);
                    if m.readonly {
                        format!("public {} {} => {};", m.ty, property, m.name)
                    } else {
                        format!(
                            "public {} {}\n{{\n    get => {};\n    set => {} = value;\n}}",
                            m.ty, property, m.name, m.name
                        )
                    }
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
            Kind::Equality => {
                if self
                    .existing
                    .iter()
                    .any(|n| n == "Equals" || n == "GetHashCode")
                {
                    return Err("Equality members already exist");
                }
                let checks = selected.iter().map(|m| format!("global::System.Collections.Generic.EqualityComparer<{}>.Default.Equals({}, other.{})", m.ty, m.name, m.name)).collect::<Vec<_>>().join(" && ");
                let hashes = selected
                    .iter()
                    .map(|m| format!("hash.Add({});", m.name))
                    .collect::<Vec<_>>()
                    .join("\n    ");
                format!(
                    "public override bool Equals(object? obj) => obj is {} other && {checks};\n\npublic override int GetHashCode()\n{{\n    var hash = new global::System.HashCode();\n    {hashes}\n    return hash.ToHashCode();\n}}",
                    self.type_name
                )
            }
            Kind::ToString => {
                if self.existing.iter().any(|n| n == "ToString") {
                    return Err("ToString already exists");
                }
                let values = selected
                    .iter()
                    .map(|m| format!("{} = {{{}}}", m.name, m.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("public override string ToString() => $\"{name} {{{{ {values} }}}}\";")
            }
            Kind::Formatting => {
                if self.existing.iter().any(|n| n == "ToString") {
                    return Err("ToString already exists");
                }
                let values = selected
                    .iter()
                    .map(|m| format!("{} = {{{}}}", m.name, m.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "public override string ToString() => ToString(null, global::System.Globalization.CultureInfo.CurrentCulture);\n\npublic string ToString(string? format, global::System.IFormatProvider? provider)\n{{\n    return format switch\n    {{\n        \"G\" or null => $\"{name} {{{{ {values} }}}}\",\n        _ => throw new global::System.FormatException($\"Unsupported format: {{format}}\")\n    }};\n}}"
                )
            }
            Kind::Implement | Kind::Override | Kind::Delegate => unreachable!(),
        };
        Ok(self.indent_code(&body))
    }

    fn indent_code(&self, body: &str) -> String {
        let unit = if self.indent.contains('\t') {
            "\t"
        } else {
            "    "
        };
        let prefix = format!("{}{}", self.indent, unit);
        let formatted = body
            .lines()
            .map(|line| format!("{prefix}{line}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("\n{formatted}\n")
    }
}

fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

fn named<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn property_name(name: &str) -> String {
    let trimmed = name.trim_start_matches('_');
    let mut chars = trimmed.chars();
    chars.next().map_or_else(
        || "Value".into(),
        |first| first.to_uppercase().collect::<String>() + chars.as_str(),
    )
}

fn parameter_name(name: &str) -> String {
    let property = property_name(name);
    let mut chars = property.chars();
    chars.next().map_or_else(
        || "value".into(),
        |first| first.to_lowercase().collect::<String>() + chars.as_str(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog(kind: Kind) -> Dialog {
        Dialog {
            doc_id: 1,
            version: 0,
            class_name: "Person".into(),
            type_name: "Person".into(),
            insert_byte: 0,
            indent: String::new(),
            kind,
            members: vec![
                Member {
                    name: "_name".into(),
                    ty: "string".into(),
                    field: true,
                    readonly: false,
                    selected: true,
                },
                Member {
                    name: "Age".into(),
                    ty: "int".into(),
                    field: false,
                    readonly: false,
                    selected: true,
                },
            ],
            targets: Vec::new(),
            existing: vec!["Age".into()],
        }
    }

    #[test]
    fn constructor_and_properties_use_selected_members() {
        let ctor = dialog(Kind::Constructor).render().unwrap();
        assert!(ctor.contains("public Person(string name, int age)"));
        assert!(ctor.contains("this._name = name;"));
        let property = dialog(Kind::Properties).render().unwrap();
        assert!(property.contains("public string Name"));
        assert!(!property.contains("public int Age"));
    }

    #[test]
    fn equality_formatting_and_to_string_are_generated() {
        let equality = dialog(Kind::Equality).render().unwrap();
        assert!(equality.contains("EqualityComparer<string>.Default.Equals(_name, other._name)"));
        assert!(equality.contains("hash.Add(Age)"));
        let display = dialog(Kind::ToString).render().unwrap();
        assert!(display.contains("_name = {_name}"));
        assert!(display.contains("Person {{ "));
        assert!(
            dialog(Kind::Formatting)
                .render()
                .unwrap()
                .contains("string? format")
        );
    }

    #[test]
    fn duplicate_named_members_are_rejected() {
        let mut d = dialog(Kind::Equality);
        d.existing.push("Equals".into());
        assert!(d.render().is_err());
    }

    #[test]
    fn picker_reads_fields_properties_and_generic_type_at_caret() {
        let source = "class Box<T>\n{\n    private readonly T _value;\n    public int Count { get; set; }\n}\n";
        let rope = rc_text::Rope::from_str(source);
        let tree = rc_syntax::parse_now(rc_syntax::Lang::CSharp, &rope, None).unwrap();
        let caret = source.find("_value").unwrap();
        let dialog = Dialog::from_source(source, &tree, caret, 7, 0).unwrap();
        assert_eq!(dialog.type_name, "Box<T>");
        assert_eq!(dialog.members.len(), 2);
        assert_eq!(dialog.members[0].name, "_value");
        assert!(dialog.members[0].readonly);
        assert!(dialog.members.iter().any(|m| m.name == "Count"));
    }

    #[test]
    fn generated_members_parse_as_csharp() {
        for kind in [
            Kind::Constructor,
            Kind::Properties,
            Kind::Equality,
            Kind::ToString,
            Kind::Formatting,
        ] {
            let code = dialog(kind).render().unwrap();
            let source = format!(
                "class Person\n{{\nprivate string _name = string.Empty;\npublic int Age {{ get; set; }}\n{code}\n}}"
            );
            let rope = rc_text::Rope::from_str(&source);
            let tree = rc_syntax::parse_now(rc_syntax::Lang::CSharp, &rope, None).unwrap();
            assert!(!tree.root_node().has_error(), "{kind:?}: {code}");
        }
    }

    #[test]
    fn picker_offers_interface_override_and_delegate_members() {
        let source = "interface IName { string Name { get; set; } void Rename(string value); }\nclass Base { public virtual int Score() => 1; }\nclass Service { public string Format(int n) => n.ToString(); }\nclass Derived : Base, IName { private Service _service; }";
        let rope = rc_text::Rope::from_str(source);
        let tree = rc_syntax::parse_now(rc_syntax::Lang::CSharp, &rope, None).unwrap();
        let caret = source.find("_service").unwrap();
        let mut dialog = Dialog::from_source(source, &tree, caret, 1, 0).unwrap();
        assert!(
            dialog
                .targets
                .iter()
                .any(|m| m.kind == Kind::Implement && m.label.contains("Rename"))
        );
        assert!(
            dialog
                .targets
                .iter()
                .any(|m| m.kind == Kind::Implement && m.label.contains("Name"))
        );
        assert!(
            dialog
                .targets
                .iter()
                .any(|m| m.kind == Kind::Override && m.label.contains("Score"))
        );
        assert!(
            dialog
                .targets
                .iter()
                .any(|m| m.kind == Kind::Delegate && m.label.contains("Format"))
        );
        for kind in [Kind::Implement, Kind::Override, Kind::Delegate] {
            dialog.kind = kind;
            let code = dialog.render().unwrap();
            assert!(code.contains("public"));
            let inserted = source.replacen(
                "private Service _service;",
                &format!("private Service _service;\n{code}"),
                1,
            );
            let tree = rc_syntax::parse_now(
                rc_syntax::Lang::CSharp,
                &rc_text::Rope::from_str(&inserted),
                None,
            )
            .unwrap();
            assert!(!tree.root_node().has_error(), "{kind:?}: {code}");
        }
    }

    #[test]
    #[ignore = "requires a .NET SDK; run with DOTNET_BIN set to the SDK executable"]
    fn generated_target_members_build_with_dotnet() {
        let source = "interface IName { string Name { get; set; } void Rename(string value); }\nclass Base { public virtual int Score() => 1; }\nclass Service { public string Format(int n) => n.ToString(); }\nclass Derived : Base, IName { private Service _service = new(); }";
        let rope = rc_text::Rope::from_str(source);
        let tree = rc_syntax::parse_now(rc_syntax::Lang::CSharp, &rope, None).unwrap();
        let caret = source.find("_service").unwrap();
        let mut dialog = Dialog::from_source(source, &tree, caret, 1, 0).unwrap();
        let mut code = String::new();
        for kind in [Kind::Implement, Kind::Override, Kind::Delegate] {
            dialog.kind = kind;
            code.push_str(&dialog.render().unwrap());
        }
        let complete = source.replacen(
            "private Service _service = new();",
            &format!("private Service _service = new();\n{code}"),
            1,
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Generate.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework><Nullable>enable</Nullable></PropertyGroup></Project>").unwrap();
        std::fs::write(dir.path().join("Generated.cs"), complete).unwrap();
        let dotnet = std::env::var("DOTNET_BIN").unwrap_or_else(|_| "dotnet".into());
        let output = std::process::Command::new(dotnet)
            .arg("build")
            .arg("--nologo")
            .arg("--verbosity")
            .arg("quiet")
            .current_dir(dir.path())
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
