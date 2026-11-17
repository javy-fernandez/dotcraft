//! C# member generation (Rider's Generate: ⌘N / Alt+Insert) from the syntax tree of the
//! active file: constructors, properties, equality and formatting members.
//!
//! The code follows the file: its indentation unit, the indentation of the type's members,
//! whether the project enables nullable reference types and implicit `using`s. Bodies are
//! written with one `\t` per nested level, replaced by the file's unit when inserted.

use crate::docs::Doc;
use rc_kit::i18n::tr;
use rc_syntax::tree_sitter::{Node, Tree};
use std::ops::Range;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Constructor,
    Properties,
    Equality,
    Formatting,
    Implement,
    Override,
    Delegate,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Constructor => tr("Constructor"),
            Self::Properties => tr("Properties"),
            Self::Equality => tr("Equality Members"),
            Self::Formatting => tr("Formatting Members"),
            Self::Implement => tr("Missing Members"),
            Self::Override => tr("Overriding Members"),
            Self::Delegate => tr("Delegating Members"),
        }
    }

    /// Members of other types (interfaces, base classes, fields' types), listed as targets.
    pub fn uses_targets(self) -> bool {
        matches!(self, Self::Implement | Self::Override | Self::Delegate)
    }

    pub fn icon(self) -> rc_kit::Ico {
        use rc_kit::ico;
        match self {
            Self::Constructor => ico::CONSTRUCTOR,
            Self::Properties => ico::PROPERTY,
            Self::Equality | Self::Formatting | Self::Override => ico::METHOD,
            Self::Implement => ico::INTERFACE,
            Self::Delegate => ico::FIELD,
        }
    }
}

/// What the type at the caret is: equality and constructors differ for each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeKind {
    Class,
    Struct,
    Record,
    RecordStruct,
}

impl TypeKind {
    fn is_value(self) -> bool {
        matches!(self, Self::Struct | Self::RecordStruct)
    }

    fn is_record(self) -> bool {
        matches!(self, Self::Record | Self::RecordStruct)
    }
}

/// How the file writes C#: nullable annotations and which `System` names need no qualifier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    /// Nullable reference types are enabled (`object?` instead of `object`).
    pub nullable: bool,
    /// `System` is imported (`using System;` or implicit usings).
    pub system: bool,
    /// `System.Collections.Generic` is imported.
    pub collections: bool,
}

impl Style {
    /// Reads the file's `using`s and `#nullable`, then the project and `Directory.Build.props`.
    pub fn detect(path: &Path, source: &str) -> Self {
        let mut nullable = None;
        let mut implicit = None;
        let mut dir = path.parent();
        let mut found_project = false;
        for _ in 0..12 {
            let Some(d) = dir else { break };
            let mut files = Vec::new();
            if !found_project && let Ok(entries) = std::fs::read_dir(d) {
                let mut projects: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "csproj")).collect();
                projects.sort();
                if let Some(project) = projects.into_iter().next() {
                    files.push(project);
                    found_project = true;
                }
            }
            files.push(d.join("Directory.Build.props"));
            for file in files {
                // The project wins over the props files above it: the first value found stays.
                let Ok(text) = std::fs::read_to_string(&file) else { continue };
                nullable = nullable.or_else(|| msbuild_property(&text, "Nullable").map(|v| v == "enable" || v == "annotations"));
                implicit = implicit.or_else(|| msbuild_property(&text, "ImplicitUsings").map(|v| v == "enable" || v == "true"));
            }
            if d.join(".git").exists() {
                break;
            }
            dir = d.parent();
        }
        let implicit = implicit.unwrap_or(false);
        let imports = |ns: &str| {
            source.lines().map(str::trim).any(|l| {
                let l = l.strip_prefix("global ").unwrap_or(l);
                l.strip_prefix("using ").is_some_and(|rest| rest.trim_end_matches(';').trim() == ns)
            })
        };
        // The last `#nullable` directive of the file decides over the project.
        let directive = source.lines().map(str::trim).filter_map(|l| l.strip_prefix("#nullable ")).last().map(|v| v.split_whitespace().next().unwrap_or_default().to_string());
        let nullable = match directive.as_deref() {
            Some("enable") => true,
            Some("disable") => false,
            _ => nullable.unwrap_or(false),
        };
        Style { nullable, system: implicit || imports("System"), collections: implicit || imports("System.Collections.Generic") }
    }

    /// A `System` type, qualified when the file does not import the namespace.
    pub fn system(&self, name: &str) -> String {
        if self.system { name.to_string() } else { format!("System.{name}") }
    }

    fn comparer(&self, ty: &str) -> String {
        let prefix = if self.collections { "" } else { "System.Collections.Generic." };
        format!("{prefix}EqualityComparer<{ty}>.Default")
    }

    /// `object?` in a nullable context, `object` otherwise.
    fn nullable_object(&self) -> &'static str {
        if self.nullable { "object?" } else { "object" }
    }
}

/// The value of `<name>` in an MSBuild file, lower-cased (the last one wins, like MSBuild).
fn msbuild_property(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let mut value = None;
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        let Some(j) = after.find(&close) else { break };
        value = Some(after[..j].trim().to_ascii_lowercase());
        rest = &after[j..];
    }
    value
}

#[derive(Clone, Debug)]
pub struct Member {
    pub name: String,
    pub ty: String,
    pub field: bool,
    pub readonly: bool,
    /// A constructor can set it: fields, auto-properties and properties with a setter.
    pub assignable: bool,
    pub selected: bool,
}

/// A member of another type to implement, override or delegate to, with its generated code.
#[derive(Clone, Debug)]
pub struct TargetMember {
    pub kind: Kind,
    pub label: String,
    pub code: String,
    pub selected: bool,
    /// The generated member's name (`this` for an indexer) and, for methods and indexers, its
    /// parameter types: what makes it clash with a member the type already has.
    pub name: String,
    pub params: Option<Vec<String>>,
    /// Where it comes from: the base type, or the field and its type.
    pub source: String,
}

/// An existing method or constructor: its name and parameter types (without spaces).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub name: String,
    pub params: Vec<String>,
}

/// Where and how the generated text goes into the type's body.
#[derive(Clone, Debug, Default)]
pub struct Insertion {
    /// Bytes replaced: the line of the closing brace, or the spaces before it when it shares
    /// a line with other code.
    pub range: Range<usize>,
    /// Indentation of a member line.
    pub indent: String,
    /// One indentation level of the file.
    pub unit: String,
    /// The closing brace shares its line: break the line before and indent the brace after.
    pub inline_close: Option<String>,
    /// The body already has members: leave a blank line before the new ones.
    pub separate: bool,
}

impl Insertion {
    /// The text to insert for `blocks` (members whose nested lines start with `\t`s).
    pub fn text(&self, blocks: &[String]) -> String {
        let lines: Vec<String> = blocks
            .join("\n\n")
            .lines()
            .map(|line| {
                if line.is_empty() {
                    return String::new();
                }
                let depth = line.chars().take_while(|c| *c == '\t').count();
                format!("{}{}{}", self.indent, self.unit.repeat(depth), &line[depth..])
            })
            .collect();
        let body = lines.join("\n");
        let separator = if self.separate { "\n" } else { "" };
        match &self.inline_close {
            Some(close_indent) => format!("\n{separator}{body}\n{close_indent}"),
            None => format!("{separator}{body}\n"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Dialog {
    pub doc_id: u64,
    pub version: u64,
    pub class_name: String,
    /// The name with its type parameters (`Box<T>`).
    pub type_name: String,
    pub type_kind: TypeKind,
    pub sealed: bool,
    pub is_abstract: bool,
    /// Declared with a primary constructor (`record Person(string Name)`, `class C(int x)`).
    pub primary_constructor: bool,
    pub insert: Insertion,
    pub style: Style,
    pub kind: Kind,
    pub members: Vec<Member>,
    pub targets: Vec<TargetMember>,
    /// Names of the type's members (fields, properties, events, methods, nested types).
    pub existing: Vec<String>,
    /// Methods and constructors of the type.
    pub methods: Vec<Signature>,
    /// Picker state: the kind is chosen first, then the members.
    pub choosing_kind: bool,
    /// Highlighted row of the current list.
    pub cursor: usize,
    /// Why the last Generate did nothing (shown in the dialog).
    pub error: Option<String>,
}

impl Dialog {
    pub fn from_doc(doc: &Doc) -> Option<Self> {
        if doc.editor.read_only || doc.path.extension()?.to_str()? != "cs" {
            return None;
        }
        let rope = doc.editor.buffer.rope();
        // A parse still running for the last keystroke would give stale offsets: parse now.
        let fresh;
        let tree = match doc.editor.syntax.tree() {
            Some(tree) if doc.editor.syntax.is_current() => tree,
            _ => {
                fresh = rc_syntax::parse_now(rc_syntax::Lang::CSharp, rope, None)?;
                &fresh
            }
        };
        let source = rope.to_string();
        let byte = rope.char_to_byte(doc.editor.caret());
        let style = Style::detect(&doc.path, &source);
        Self::from_source(&source, tree, byte, doc.editor.id, doc.editor.buffer.version(), &doc.editor.indent.unit, style)
    }

    pub fn from_source(source: &str, tree: &Tree, byte: usize, doc_id: u64, version: u64, unit: &str, style: Style) -> Option<Self> {
        let node = type_at(tree, byte)?;
        let name = text(node.child_by_field_name("name")?, source).to_string();
        let type_name = format!("{}{}", name, named(node, "type_parameter_list").map(|n| text(n, source)).unwrap_or_default());
        let mods = modifiers(node, source);
        let type_kind = match node.kind() {
            "struct_declaration" => TypeKind::Struct,
            "record_declaration" if has_token(node, "struct") => TypeKind::RecordStruct,
            "record_declaration" => TypeKind::Record,
            _ => TypeKind::Class,
        };
        let body = node.child_by_field_name("body")?;
        let insert = insertion(source, node, body, unit);
        let mut dialog = Self {
            doc_id,
            version,
            class_name: name,
            type_name,
            type_kind,
            sealed: mods.iter().any(|m| m == "sealed"),
            is_abstract: mods.iter().any(|m| m == "abstract"),
            primary_constructor: named(node, "parameter_list").is_some(),
            insert,
            style,
            kind: Kind::Constructor,
            members: Vec::new(),
            targets: Vec::new(),
            existing: Vec::new(),
            methods: Vec::new(),
            choosing_kind: true,
            cursor: 0,
            error: None,
        };
        // Record parameters are properties.
        if type_kind.is_record()
            && let Some(params) = named(node, "parameter_list")
        {
            let mut cursor = params.walk();
            for p in params.named_children(&mut cursor).filter(|p| p.kind() == "parameter") {
                if let (Some(n), Some(t)) = (p.child_by_field_name("name"), p.child_by_field_name("type")) {
                    dialog.existing.push(text(n, source).to_string());
                    dialog.members.push(Member { name: text(n, source).into(), ty: text(t, source).into(), field: false, readonly: true, assignable: false, selected: true });
                }
            }
        }
        let mut part = Part::default();
        part.read_body(source, body);
        dialog.merge(part);
        dialog.targets = crate::generate_targets::collect(tree, source, node, &dialog.members, &dialog.known());
        Some(dialog)
    }

    /// Adds the members of another part of the type (`partial`), and drops the targets they
    /// already implement.
    pub fn merge(&mut self, part: Part) {
        self.members.extend(part.members);
        self.existing.extend(part.existing);
        self.methods.extend(part.methods);
        let known = self.known();
        self.targets.retain(|t| !known.has(&t.name, t.params.as_deref()));
    }

    /// What the type already declares, for the target generators.
    pub fn known(&self) -> crate::generate_targets::Known {
        crate::generate_targets::Known { existing: self.existing.clone(), methods: self.methods.clone(), style: self.style }
    }

    /// The kinds that make sense for this type, in Rider's order.
    pub fn kinds(&self) -> Vec<Kind> {
        let mut kinds = Vec::new();
        if !self.primary_constructor {
            kinds.push(Kind::Constructor);
        }
        if self.members.iter().any(|m| m.field && self.property_candidate(m)) {
            kinds.push(Kind::Properties);
        }
        // Records synthesize Equals and GetHashCode.
        if !self.type_kind.is_record() {
            kinds.push(Kind::Equality);
        }
        kinds.push(Kind::Formatting);
        kinds.extend([Kind::Implement, Kind::Override, Kind::Delegate]);
        kinds
    }

    fn property_candidate(&self, m: &Member) -> bool {
        let property = property_name(&m.name);
        property != m.name && !self.existing.contains(&property)
    }

    /// A property backed by a listed field (`_name` and `Name`): the field stands for both.
    fn backed(&self, m: &Member) -> bool {
        !m.field && self.members.iter().any(|f| f.field && parameter_name(&f.name) == parameter_name(&m.name))
    }

    pub fn candidates(&self) -> impl Iterator<Item = (usize, &Member)> {
        self.members.iter().enumerate().filter(|(_, m)| match self.kind {
            Kind::Properties => m.field && self.property_candidate(m),
            Kind::Constructor => m.assignable && !self.backed(m),
            Kind::Equality | Kind::Formatting => !self.backed(m),
            Kind::Implement | Kind::Override | Kind::Delegate => false,
        })
    }

    /// The edit that inserts the selected members: a byte range and its new text.
    pub fn edit(&self) -> Result<(Range<usize>, String), String> {
        let blocks = self.blocks()?;
        Ok((self.insert.range.clone(), self.insert.text(&blocks)))
    }

    fn blocks(&self) -> Result<Vec<String>, String> {
        if self.kind.uses_targets() {
            let code: Vec<String> = self.targets.iter().filter(|m| m.kind == self.kind && m.selected).map(|m| m.code.clone()).collect();
            if code.is_empty() {
                return Err(tr("Select at least one member").into());
            }
            return Ok(code);
        }
        let selected: Vec<&Member> = self.candidates().filter(|(_, m)| m.selected).map(|(_, m)| m).collect();
        if selected.is_empty() && self.kind != Kind::Constructor {
            return Err(tr("Select at least one member").into());
        }
        let style = &self.style;
        let ty = &self.type_name;
        Ok(match self.kind {
            Kind::Constructor => {
                let types: Vec<String> = selected.iter().map(|m| normalize_type(&m.ty)).collect();
                if self.methods.iter().any(|s| s.name == self.class_name && s.params == types) {
                    return Err(tr("A constructor with these parameters already exists").into());
                }
                let params = selected.iter().map(|m| format!("{} {}", m.ty, parameter_name(&m.name))).collect::<Vec<_>>().join(", ");
                let access = if self.is_abstract { "protected" } else { "public" };
                let assignments: String = selected
                    .iter()
                    .map(|m| {
                        let param = parameter_name(&m.name);
                        let target = if param.trim_start_matches('@') == m.name { format!("this.{}", m.name) } else { m.name.clone() };
                        format!("\t{target} = {param};\n")
                    })
                    .collect();
                vec![format!("{access} {}({params})\n{{\n{assignments}}}", self.class_name)]
            }
            Kind::Properties => selected
                .iter()
                .map(|m| {
                    let property = property_name(&m.name);
                    if m.readonly {
                        format!("public {} {property} => {};", m.ty, m.name)
                    } else {
                        format!("public {} {property}\n{{\n\tget => {};\n\tset => {} = value;\n}}", m.ty, m.name, m.name)
                    }
                })
                .collect(),
            Kind::Equality => {
                if self.methods.iter().any(|s| (s.name == "Equals" && s.params.len() == 1) || (s.name == "GetHashCode" && s.params.is_empty())) {
                    return Err(tr("Equals or GetHashCode already exists").into());
                }
                let checks = selected.iter().map(|m| equals_expression(&m.ty, &m.name, style)).collect::<Vec<_>>().join(" && ");
                let names: Vec<&str> = selected.iter().map(|m| m.name.as_str()).collect();
                let hash = hash_code(&names, style);
                let object = style.nullable_object();
                let mut blocks = Vec::new();
                if self.type_kind.is_value() {
                    blocks.push(format!("public bool Equals({ty} other) => {checks};"));
                    blocks.push(format!("public override bool Equals({object} obj) => obj is {ty} other && Equals(other);"));
                    blocks.push(hash);
                    blocks.push(format!("public static bool operator ==({ty} left, {ty} right) => left.Equals(right);"));
                    blocks.push(format!("public static bool operator !=({ty} left, {ty} right) => !left.Equals(right);"));
                } else if self.sealed {
                    blocks.push(format!("private bool Equals({ty} other) => {checks};"));
                    blocks.push(format!("public override bool Equals({object} obj) => ReferenceEquals(this, obj) || obj is {ty} other && Equals(other);"));
                    blocks.push(hash);
                } else {
                    // Derived types are never equal to their base: compare the runtime types.
                    blocks.push(format!("protected bool Equals({ty} other) => {checks};"));
                    blocks.push(format!(
                        "public override bool Equals({object} obj)\n{{\n\tif (obj is null) return false;\n\tif (ReferenceEquals(this, obj)) return true;\n\treturn obj.GetType() == GetType() && Equals(({ty})obj);\n}}"
                    ));
                    blocks.push(hash);
                }
                blocks
            }
            Kind::Formatting => {
                if self.methods.iter().any(|s| s.name == "ToString" && s.params.is_empty()) {
                    return Err(tr("ToString already exists").into());
                }
                let values = selected.iter().map(|m| format!("{{nameof({0})}}: {{{0}}}", m.name)).collect::<Vec<_>>().join(", ");
                vec![format!("public override string ToString() => $\"{values}\";")]
            }
            Kind::Implement | Kind::Override | Kind::Delegate => unreachable!(),
        })
    }
}

/// What one declaration of a type has: its fields and properties, and every member name.
#[derive(Clone, Debug, Default)]
pub struct Part {
    pub members: Vec<Member>,
    pub existing: Vec<String>,
    pub methods: Vec<Signature>,
}

impl Part {
    /// The members of the `partial` declarations of `name` in another file.
    pub fn of_partial(source: &str, name: &str, arity: usize) -> Self {
        let mut part = Part::default();
        let Some(tree) = rc_syntax::parse_now(rc_syntax::Lang::CSharp, &rc_text::Rope::from_str(source), None) else { return part };
        let mut types = Vec::new();
        crate::generate_targets::walk_types(tree.root_node(), source, &mut types);
        for node in types {
            let params = named(node, "type_parameter_list").map_or(0, |list| {
                let mut cursor = list.walk();
                list.named_children(&mut cursor).filter(|n| n.kind() == "type_parameter").count()
            });
            let same = node.child_by_field_name("name").is_some_and(|n| text(n, source) == name) && params == arity;
            if same
                && modifiers(node, source).iter().any(|m| m == "partial")
                && let Some(body) = node.child_by_field_name("body")
            {
                part.read_body(source, body);
            }
        }
        part
    }

    fn read_body(&mut self, source: &str, body: Node<'_>) {
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            let mods = modifiers(child, source);
            let is_static = mods.iter().any(|m| m == "static" || m == "const");
            match child.kind() {
                "field_declaration" => {
                    let Some(decl) = named(child, "variable_declaration") else { continue };
                    let Some(ty) = decl.child_by_field_name("type") else { continue };
                    let mut c = decl.walk();
                    for variable in decl.named_children(&mut c).filter(|n| n.kind() == "variable_declarator") {
                        let Some(name) = variable.child_by_field_name("name") else { continue };
                        self.existing.push(text(name, source).into());
                        if !is_static {
                            self.members.push(Member {
                                name: text(name, source).into(),
                                ty: text(ty, source).into(),
                                field: true,
                                readonly: mods.iter().any(|m| m == "readonly"),
                                assignable: true,
                                selected: true,
                            });
                        }
                    }
                }
                "property_declaration" => {
                    let (Some(name), Some(ty)) = (child.child_by_field_name("name"), child.child_by_field_name("type")) else { continue };
                    let name = text(name, source).to_string();
                    self.existing.push(name.clone());
                    // Explicit interface implementations are not members of the type's surface.
                    if is_static || named(child, "explicit_interface_specifier").is_some() {
                        continue;
                    }
                    let accessors = accessors(child, source);
                    let auto = !accessors.is_empty() && accessors.iter().all(|(_, body)| !body);
                    let setter = accessors.iter().any(|(n, _)| n == "set" || n == "init");
                    let expression_bodied = child.child_by_field_name("value").is_some_and(|v| v.kind() == "arrow_expression_clause");
                    self.members.push(Member {
                        name,
                        ty: text(ty, source).into(),
                        field: false,
                        readonly: !setter,
                        assignable: !expression_bodied && (auto || setter),
                        selected: true,
                    });
                }
                "method_declaration" | "constructor_declaration" => {
                    let Some(name) = child.child_by_field_name("name") else { continue };
                    let name = text(name, source).to_string();
                    self.existing.push(name.clone());
                    let params = child.child_by_field_name("parameters").map(|p| parameter_types(p, source)).unwrap_or_default();
                    self.methods.push(Signature { name, params });
                }
                "indexer_declaration" => {
                    let params = child.child_by_field_name("parameters").map(|p| parameter_types(p, source)).unwrap_or_default();
                    self.methods.push(Signature { name: "this".into(), params });
                }
                "event_field_declaration" => {
                    let Some(decl) = named(child, "variable_declaration") else { continue };
                    let mut c = decl.walk();
                    for variable in decl.named_children(&mut c).filter(|n| n.kind() == "variable_declarator") {
                        if let Some(name) = variable.child_by_field_name("name") {
                            self.existing.push(text(name, source).into());
                        }
                    }
                }
                _ => {
                    if let Some(name) = child.child_by_field_name("name") {
                        self.existing.push(text(name, source).into());
                    }
                }
            }
        }
    }

}

/// The innermost class, struct or record around `byte`; None inside an interface or enum.
fn type_at(tree: &Tree, byte: usize) -> Option<Node<'_>> {
    let mut node = tree.root_node().descendant_for_byte_range(byte, byte)?;
    loop {
        match node.kind() {
            "class_declaration" | "struct_declaration" | "record_declaration" => return Some(node),
            "interface_declaration" | "enum_declaration" => return None,
            _ => node = node.parent()?,
        }
    }
}

/// Where new members go: before the closing brace, indented like the type's members.
fn insertion(source: &str, node: Node<'_>, body: Node<'_>, unit: &str) -> Insertion {
    let line_indent = |byte: usize| -> Option<String> {
        let start = source[..byte].rfind('\n').map_or(0, |i| i + 1);
        let prefix = &source[start..byte];
        prefix.chars().all(|c| c == ' ' || c == '\t').then(|| prefix.to_string())
    };
    let type_indent = {
        let start = source[..node.start_byte()].rfind('\n').map_or(0, |i| i + 1);
        source[start..node.start_byte()].chars().take_while(|c| *c == ' ' || *c == '\t').collect::<String>()
    };
    let mut cursor = body.walk();
    let members: Vec<Node<'_>> = body.named_children(&mut cursor).collect();
    let indent = members.iter().find_map(|m| line_indent(m.start_byte())).unwrap_or_else(|| format!("{type_indent}{unit}"));
    let close = body.end_byte().saturating_sub(1);
    let (range, inline_close) = match line_indent(close) {
        Some(prefix) => (close - prefix.len()..close - prefix.len(), None),
        None => {
            let start = source[..close].trim_end_matches([' ', '\t']).len();
            (start..close, Some(type_indent))
        }
    };
    let separate = !members.is_empty();
    Insertion { range, indent, unit: unit.to_string(), inline_close, separate }
}

pub(crate) fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

pub(crate) fn named<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|child| child.kind() == kind)
}

/// The declaration's modifiers (`public`, `static`, `virtual`…), in order.
pub(crate) fn modifiers(node: Node<'_>, source: &str) -> Vec<String> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).filter(|c| c.kind() == "modifier").map(|c| text(c, source).to_string()).collect()
}

/// Whether `node` has the anonymous token `token` as a direct child (`record struct`).
fn has_token(node: Node<'_>, token: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|c| !c.is_named() && c.kind() == token)
}

/// The accessors of a property, indexer or event: name (`get`, `set`, `init`…) and whether it
/// has a body.
pub(crate) fn accessors(node: Node<'_>, source: &str) -> Vec<(String, bool)> {
    let Some(list) = node.child_by_field_name("accessors") else { return Vec::new() };
    let mut cursor = list.walk();
    list.named_children(&mut cursor)
        .filter(|a| a.kind() == "accessor_declaration")
        .filter_map(|a| Some((text(a.child_by_field_name("name")?, source).to_string(), a.child_by_field_name("body").is_some())))
        .collect()
}

/// The parameter types of a parameter list, with their `ref`/`out`/`in`/`params` modifiers.
pub(crate) fn parameter_types(params: Node<'_>, source: &str) -> Vec<String> {
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .filter(|p| p.kind() == "parameter")
        .map(|p| {
            let ty = p.child_by_field_name("type").map(|t| normalize_type(text(t, source))).unwrap_or_default();
            match modifiers(p, source).into_iter().find(|m| matches!(m.as_str(), "ref" | "out" | "in")) {
                Some(m) => format!("{m} {ty}"),
                None => ty,
            }
        })
        .collect()
}

/// A type without spaces, to compare signatures.
pub(crate) fn normalize_type(ty: &str) -> String {
    ty.chars().filter(|c| !c.is_whitespace()).collect()
}

/// `a == other.a` for built-in types, the type's default comparer otherwise.
fn equals_expression(ty: &str, name: &str, style: &Style) -> String {
    const EQUALITY_OPERATOR: &[&str] = &[
        "bool", "byte", "sbyte", "char", "short", "ushort", "int", "uint", "long", "ulong", "nint", "nuint", "decimal", "string", "Guid", "DateTime", "DateTimeOffset", "TimeSpan",
    ];
    let bare = normalize_type(ty);
    let base = bare.trim_end_matches('?');
    if EQUALITY_OPERATOR.contains(&base) {
        format!("{name} == other.{name}")
    } else if matches!(base, "float" | "double") && !bare.ends_with('?') {
        // NaN equals NaN with Equals, not with ==.
        format!("{name}.Equals(other.{name})")
    } else {
        format!("{}.Equals({name}, other.{name})", style.comparer(ty))
    }
}

/// `GetHashCode` with `HashCode.Combine` (up to eight values) or a `HashCode` builder.
fn hash_code(names: &[&str], style: &Style) -> String {
    let hash = style.system("HashCode");
    if names.len() <= 8 {
        return format!("public override int GetHashCode() => {hash}.Combine({});", names.join(", "));
    }
    let adds: String = names.iter().map(|n| format!("\thash.Add({n});\n")).collect();
    format!("public override int GetHashCode()\n{{\n\tvar hash = new {hash}();\n{adds}\treturn hash.ToHashCode();\n}}")
}

/// `_name`, `m_name` or `name` → `Name`.
pub(crate) fn property_name(name: &str) -> String {
    let trimmed = name.strip_prefix("m_").unwrap_or(name).trim_start_matches('_');
    let mut chars = trimmed.chars();
    chars.next().map_or_else(|| "Value".into(), |first| first.to_uppercase().collect::<String>() + chars.as_str())
}

/// `_name` or `Name` → `name`; C# keywords get an `@` (`@class`).
pub(crate) fn parameter_name(name: &str) -> String {
    let property = property_name(name);
    let mut chars = property.chars();
    let camel = chars.next().map_or_else(|| "value".into(), |first| first.to_lowercase().collect::<String>() + chars.as_str());
    if KEYWORDS.contains(&camel.as_str()) { format!("@{camel}") } else { camel }
}

const KEYWORDS: &[&str] = &[
    "abstract", "as", "base", "bool", "break", "byte", "case", "catch", "char", "checked", "class", "const", "continue", "decimal", "default", "delegate", "do", "double", "else", "enum", "event",
    "explicit", "extern", "false", "finally", "fixed", "float", "for", "foreach", "goto", "if", "implicit", "in", "int", "interface", "internal", "is", "lock", "long", "namespace", "new", "null",
    "object", "operator", "out", "override", "params", "private", "protected", "public", "readonly", "ref", "return", "sbyte", "sealed", "short", "sizeof", "stackalloc", "static", "string", "struct",
    "switch", "this", "throw", "true", "try", "typeof", "uint", "ulong", "unchecked", "unsafe", "ushort", "using", "virtual", "void", "volatile", "while",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Tree {
        rc_syntax::parse_now(rc_syntax::Lang::CSharp, &rc_text::Rope::from_str(source), None).unwrap()
    }

    fn dialog_at(source: &str, at: &str, style: Style) -> Dialog {
        let tree = parse(source);
        Dialog::from_source(source, &tree, source.find(at).unwrap(), 1, 0, "    ", style).unwrap()
    }

    /// Applies the dialog's edit and checks the result parses.
    fn apply(source: &str, dialog: &Dialog) -> String {
        let (range, text) = dialog.edit().unwrap();
        let out = format!("{}{}{}", &source[..range.start], text, &source[range.end..]);
        assert!(!parse(&out).root_node().has_error(), "{out}");
        out
    }

    const PERSON: &str = "class Person\n{\n    private string _name = string.Empty;\n    public int Age { get; set; }\n}\n";

    #[test]
    fn constructor_assigns_fields_and_auto_properties() {
        let mut d = dialog_at(PERSON, "_name", Style::default());
        d.kind = Kind::Constructor;
        let out = apply(PERSON, &d);
        assert!(out.contains("    public int Age { get; set; }\n\n    public Person(string name, int age)\n    {\n        _name = name;\n        Age = age;\n    }\n}\n"), "{out}");
        // The same parameters again: refused.
        let tree = parse(&out);
        let mut again = Dialog::from_source(&out, &tree, out.find("_name").unwrap(), 1, 0, "    ", Style::default()).unwrap();
        again.kind = Kind::Constructor;
        assert!(again.edit().is_err());
        // An overload is fine.
        again.members[1].selected = false;
        assert!(again.edit().is_ok());
    }

    #[test]
    fn constructor_uses_this_and_escapes_keywords() {
        let source = "abstract class Node\n{\n    private int name;\n    private string _class;\n    public int Computed => 1;\n    public static int Count;\n}\n";
        let mut d = dialog_at(source, "name", Style::default());
        d.kind = Kind::Constructor;
        assert_eq!(d.candidates().count(), 2, "computed and static members are not parameters");
        let out = apply(source, &d);
        assert!(out.contains("protected Node(int name, string @class)\n    {\n        this.name = name;\n        _class = @class;\n    }"), "{out}");
    }

    #[test]
    fn properties_wrap_fields_without_one() {
        let source = "class Box<T>\n{\n    private readonly T _value;\n    private int _count;\n    private string Label;\n    public int Count => _count;\n}\n";
        let mut d = dialog_at(source, "_value", Style::default());
        assert_eq!(d.type_name, "Box<T>");
        d.kind = Kind::Properties;
        let names: Vec<_> = d.candidates().map(|(_, m)| m.name.clone()).collect();
        assert_eq!(names, ["_value"], "Count exists and Label would clash with the field");
        let out = apply(source, &d);
        assert!(out.contains("    public T Value => _value;\n}"), "{out}");
    }

    #[test]
    fn equality_for_classes_structs_and_sealed_types() {
        let style = Style { nullable: true, system: true, collections: true };
        let mut d = dialog_at(PERSON, "_name", style);
        d.kind = Kind::Equality;
        let out = apply(PERSON, &d);
        assert!(out.contains("protected bool Equals(Person other) => _name == other._name && Age == other.Age;"), "{out}");
        assert!(out.contains("public override bool Equals(object? obj)\n    {\n        if (obj is null) return false;"), "{out}");
        assert!(out.contains("return obj.GetType() == GetType() && Equals((Person)obj);"), "{out}");
        assert!(out.contains("public override int GetHashCode() => HashCode.Combine(_name, Age);"), "{out}");

        let point = "struct Point<T>\n{\n    public double X;\n    public T Tag;\n}\n";
        let mut d = dialog_at(point, "X", Style::default());
        d.kind = Kind::Equality;
        let out = apply(point, &d);
        assert!(out.contains("public bool Equals(Point<T> other) => X.Equals(other.X) && System.Collections.Generic.EqualityComparer<T>.Default.Equals(Tag, other.Tag);"), "{out}");
        assert!(out.contains("public override bool Equals(object obj) => obj is Point<T> other && Equals(other);"), "{out}");
        assert!(out.contains("System.HashCode.Combine(X, Tag)"), "{out}");
        assert!(out.contains("public static bool operator ==(Point<T> left, Point<T> right)"), "{out}");

        let sealed = "sealed class Id\n{\n    private readonly System.Guid _value;\n}\n";
        let mut d = dialog_at(sealed, "_value", style);
        d.kind = Kind::Equality;
        let out = apply(sealed, &d);
        assert!(out.contains("private bool Equals(Id other) =>"), "{out}");
        assert!(out.contains("ReferenceEquals(this, obj) || obj is Id other && Equals(other);"), "{out}");
    }

    #[test]
    fn records_offer_neither_equality_nor_a_second_primary_constructor() {
        let source = "public record Person(string Name, int Age)\n{\n    public string? Nickname { get; init; }\n}\n";
        let mut d = dialog_at(source, "Nickname", Style::default());
        assert_eq!(d.type_kind, TypeKind::Record);
        assert_eq!(d.kinds(), [Kind::Formatting, Kind::Implement, Kind::Override, Kind::Delegate]);
        d.kind = Kind::Formatting;
        let out = apply(source, &d);
        assert!(out.contains("public override string ToString() => $\"{nameof(Name)}: {Name}, {nameof(Age)}: {Age}, {nameof(Nickname)}: {Nickname}\";"), "{out}");
        let record_struct = "record struct Point\n{\n    public int X { get; init; }\n}\n";
        let d = dialog_at(record_struct, "X", Style::default());
        assert_eq!(d.type_kind, TypeKind::RecordStruct);
        assert!(!d.kinds().contains(&Kind::Equality));
        assert!(d.kinds().contains(&Kind::Constructor));
    }

    #[test]
    fn duplicate_members_are_refused() {
        let source = "class A\n{\n    int _x;\n    public override string ToString() => \"\";\n    public override bool Equals(object obj) => false;\n}\n";
        let mut d = dialog_at(source, "_x", Style::default());
        d.kind = Kind::Formatting;
        assert!(d.edit().is_err());
        d.kind = Kind::Equality;
        assert!(d.edit().is_err());
    }

    #[test]
    fn members_follow_the_file_indentation() {
        // Tabs, a nested type and a closing brace indented by the namespace.
        let source = "namespace N\n{\n\tclass Outer\n\t{\n\t\tclass Inner\n\t\t{\n\t\t\tint _a;\n\t\t}\n\t}\n}\n";
        let tree = parse(source);
        let mut d = Dialog::from_source(source, &tree, source.find("_a").unwrap(), 1, 0, "\t", Style::default()).unwrap();
        assert_eq!(d.class_name, "Inner");
        d.kind = Kind::Constructor;
        let out = apply(source, &d);
        assert!(out.contains("\t\t\tint _a;\n\n\t\t\tpublic Inner(int a)\n\t\t\t{\n\t\t\t\t_a = a;\n\t\t\t}\n\t\t}\n\t}\n}\n"), "{out:?}");

        // Two-space files and an empty body: no blank line after the brace.
        let source = "class Empty\n{\n}\n";
        let tree = parse(source);
        let mut d = Dialog::from_source(source, &tree, 7, 1, 0, "  ", Style::default()).unwrap();
        d.kind = Kind::Constructor;
        assert_eq!(apply(source, &d), "class Empty\n{\n  public Empty()\n  {\n  }\n}\n");

        // A body on one line is opened up.
        let source = "class Point { int _x; }\n";
        let mut d = dialog_at(source, "_x", Style::default());
        d.kind = Kind::Constructor;
        assert_eq!(apply(source, &d), "class Point { int _x;\n\n    public Point(int x)\n    {\n        _x = x;\n    }\n}\n");
    }

    #[test]
    fn style_reads_the_project_and_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Directory.Build.props"), "<Project><PropertyGroup><Nullable>disable</Nullable></PropertyGroup></Project>").unwrap();
        std::fs::create_dir(dir.path().join("App")).unwrap();
        std::fs::write(dir.path().join("App/App.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings></PropertyGroup></Project>").unwrap();
        let file = dir.path().join("App/A.cs");
        assert_eq!(Style::detect(&file, "class A {}"), Style { nullable: true, system: true, collections: true });
        assert!(!Style::detect(&file, "#nullable disable\nclass A {}").nullable);
        let lib = dir.path().join("Lib/B.cs");
        std::fs::create_dir(dir.path().join("Lib")).unwrap();
        std::fs::write(dir.path().join("Lib/Lib.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\"></Project>").unwrap();
        assert_eq!(Style::detect(&lib, "using System;\nclass B {}"), Style { nullable: false, system: true, collections: false });
    }

    #[test]
    fn other_parts_of_a_partial_type_add_members_and_hide_targets() {
        let source = "interface INamed { string Name { get; } void Rename(string name); }\npartial class Person : INamed\n{\n    private int _age;\n}\n";
        let mut d = dialog_at(source, "_age", Style::default());
        assert_eq!(d.targets.len(), 2);
        let other = "namespace N;\npartial class Person\n{\n    private string _name = \"\";\n    public string Name => _name;\n}\nclass Person2 { int _x; }\n";
        d.merge(Part::of_partial(other, "Person", 0));
        assert_eq!(d.members.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["_age", "_name", "Name"]);
        assert_eq!(d.targets.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Rename"]);
        assert!(Part::of_partial(other, "Person", 1).members.is_empty(), "Person<T> is another type");
    }

    #[test]
    fn interfaces_and_enums_are_not_targets() {
        let source = "interface I\n{\n    void M();\n}\n";
        let tree = parse(source);
        assert!(Dialog::from_source(source, &tree, source.find("M()").unwrap(), 1, 0, "    ", Style::default()).is_none());
    }

    const TARGETS: &str = "interface IName { string Name { get; set; } void Rename(string value); event System.EventHandler Changed; string this[int i] { get; set; } }\nclass Base { public virtual int Score() => 1; }\nclass Service { public string Format(int n) => n.ToString(); }\nclass Derived : Base, IName { private Service _service = new(); }";

    #[test]
    fn picker_offers_interface_override_and_delegate_members() {
        let mut d = dialog_at(TARGETS, "_service", Style::default());
        let has = |kind: Kind, label: &str| d.targets.iter().any(|m| m.kind == kind && m.label.contains(label));
        for label in ["Rename", "Name", "Changed", "this"] {
            assert!(has(Kind::Implement, label), "{label}");
        }
        assert!(has(Kind::Override, "Score"));
        assert!(has(Kind::Delegate, "Format"));
        for kind in [Kind::Implement, Kind::Override, Kind::Delegate] {
            d.kind = kind;
            let out = apply(TARGETS, &d);
            assert!(out.contains("public"), "{kind:?}: {out}");
        }
    }

    /// Every kind, one after the other, into a class, a struct and a record of a nullable project
    /// that treats warnings as errors.
    #[test]
    #[ignore = "requires a .NET SDK; run with DOTNET_BIN set to the SDK executable"]
    fn generated_members_build_with_dotnet() {
        let style = Style { nullable: true, system: true, collections: true };
        let mut source = format!("{TARGETS}\nstruct Point<T>\n{{\n    private double _x;\n    public T? Tag {{ get; set; }}\n}}\nrecord Person(string Name)\n{{\n    private readonly int _age;\n}}\ninterface IRepository<out T> where T : class\n{{\n    T? Find<U>(U key) where U : notnull;\n    string Label {{ get; init; }}\n}}\nsealed class Repo : IRepository<string>\n{{\n    private string _label = \"\";\n}}\n");
        for (at, kinds) in [
            ("_service", &[Kind::Implement, Kind::Override, Kind::Delegate, Kind::Equality, Kind::Formatting][..]),
            ("_x", &[Kind::Constructor, Kind::Properties, Kind::Equality, Kind::Formatting][..]),
            ("_age", &[Kind::Constructor, Kind::Properties, Kind::Formatting][..]),
            ("_label", &[Kind::Implement, Kind::Constructor, Kind::Equality][..]),
        ] {
            for &kind in kinds {
                let mut d = dialog_at(&source, at, style);
                if !d.kinds().contains(&kind) {
                    continue;
                }
                d.kind = kind;
                source = apply(&source, &d);
            }
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Generate.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings><TreatWarningsAsErrors>true</TreatWarningsAsErrors><NoWarn>CS0067;CS0169;CS0414;CS0649</NoWarn></PropertyGroup></Project>").unwrap();
        std::fs::write(dir.path().join("Generated.cs"), &source).unwrap();
        let dotnet = std::env::var("DOTNET_BIN").unwrap_or_else(|_| "dotnet".into());
        let output = std::process::Command::new(dotnet).args(["build", "--nologo", "-v", "q", "-p:UseSharedCompilation=false"]).current_dir(dir.path()).env("DOTNET_CLI_TELEMETRY_OPTOUT", "1").output().unwrap();
        assert!(output.status.success(), "{source}\n{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    fn generic_interface_arguments_are_substituted() {
        let source = "interface IRepository<T> { T Find(T value); } class App : IRepository<string> { }";
        let mut d = dialog_at(source, "App :", Style::default());
        d.kind = Kind::Implement;
        let out = apply(source, &d);
        assert!(out.contains("string Find(string value)"), "{out}");
    }
}
