//! Package items of MSBuild project files as the editor sees them: the attribute under the caret
//! while typing (the XML may be incomplete), the `PackageReference`/`PackageVersion` items with
//! the byte ranges of their ids and versions, version ordering for completion, and where a
//! NuGet message (NU1xxx) belongs in a file.

use std::ops::Range;
use std::path::Path;

use crate::nuget::compare_versions;

/// Items that name a package in `Include`.
pub const PACKAGE_ELEMENTS: &[&str] = &["PackageReference", "PackageVersion", "GlobalPackageReference"];

/// Project files and the MSBuild files that can hold package items.
pub fn is_package_file(path: &Path) -> bool {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()) else { return false };
    name.ends_with(".csproj") || name.ends_with(".fsproj") || name.ends_with(".vbproj") || matches!(name.as_str(), "directory.packages.props" | "directory.build.props" | "directory.build.targets")
}

/// `Directory.Packages.props`.
pub fn is_central_file(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(crate::cpm::CENTRAL_FILE))
}

/// The attribute of a package item that holds the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageAttr {
    Include,
    Version,
    VersionOverride,
}

/// Where the caret is inside a package item's attribute value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XmlContext {
    /// `PackageReference`, `PackageVersion` or `GlobalPackageReference` (as written).
    pub element: String,
    pub attr: PackageAttr,
    /// Byte range of the value (inside the quotes; up to the end of the word when unterminated).
    pub value: Range<usize>,
    /// Text of the value before the caret.
    pub typed: String,
    /// The item's `Include` (for version completion).
    pub package_id: Option<String>,
}

/// The package attribute value under `caret` (a byte offset), from the text of the tag only, so
/// it works while the file is not valid XML.
pub fn context_at(text: &str, caret: usize) -> Option<XmlContext> {
    let caret = floor_boundary(text, caret.min(text.len()));
    let before = &text[..caret];
    let lt = before.rfind('<')?;
    if before[lt..].contains('>') {
        return None;
    }
    let tag_end = text[lt + 1..].find(['<', '>']).map_or(text.len(), |i| lt + 1 + i);
    let tag = &text[lt + 1..tag_end];
    let name_len = tag.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(tag.len());
    let element = &tag[..name_len];
    if !PACKAGE_ELEMENTS.iter().any(|e| e.eq_ignore_ascii_case(element)) {
        return None;
    }
    let attrs = attributes(text, lt + 1 + name_len, tag_end);
    let (name, value) = attrs.iter().find(|(_, v)| v.start <= caret && caret <= v.end)?;
    let attr = match name.to_ascii_lowercase().as_str() {
        "include" => PackageAttr::Include,
        "version" => PackageAttr::Version,
        "versionoverride" => PackageAttr::VersionOverride,
        _ => return None,
    };
    let package_id = attrs.iter().find(|(n, _)| n.eq_ignore_ascii_case("Include")).map(|(_, v)| text[v.clone()].trim().to_string()).filter(|s| !s.is_empty());
    Some(XmlContext { element: element.to_string(), attr, value: value.clone(), typed: text[value.start..caret].to_string(), package_id })
}

/// `name="value"` pairs between `from` and `end`: (name, value range).
fn attributes(text: &str, from: usize, end: usize) -> Vec<(String, Range<usize>)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = from;
    let skip_ws = |mut i: usize| {
        while i < end && b[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    loop {
        i = skip_ws(i);
        let start = i;
        while i < end && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'/' | b'"' | b'\'') {
            i += 1;
        }
        if i == start {
            // `/` of an empty-element tag, or a stray character.
            if i >= end {
                break;
            }
            i += 1;
            continue;
        }
        let name = text[start..i].to_string();
        i = skip_ws(i);
        if i >= end || b[i] != b'=' {
            continue;
        }
        i = skip_ws(i + 1);
        if i >= end || !matches!(b[i], b'"' | b'\'') {
            continue;
        }
        let q = b[i];
        let vs = i + 1;
        match b[vs..end].iter().position(|&c| c == q) {
            Some(n) => {
                out.push((name, vs..vs + n));
                i = vs + n + 1;
            }
            None => {
                // Unterminated: the value runs to the end of the word.
                let n = b[vs..end].iter().position(|&c| c.is_ascii_whitespace() || c == b'/').unwrap_or(end - vs);
                out.push((name, vs..vs + n));
                break;
            }
        }
    }
    out
}

fn floor_boundary(text: &str, mut i: usize) -> usize {
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// A package item of a parsed file, with byte ranges in its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageItem {
    pub element: String,
    pub id: String,
    /// The `Include` value.
    pub id_range: Range<usize>,
    pub version: Option<String>,
    /// The version's value (attribute value or element text).
    pub version_range: Option<Range<usize>>,
    /// The whole `Version="…"` attribute or `<Version>` element.
    pub version_span: Option<Range<usize>>,
    pub version_override: Option<String>,
    pub override_range: Option<Range<usize>>,
    /// Condition of the item or of its `ItemGroup`.
    pub condition: Option<String>,
    /// Zero-based line of the item's start.
    pub line: usize,
    /// Byte range of the whole element.
    pub range: Range<usize>,
}

/// The package items of `text`; `None` when it is not well-formed XML (while typing).
pub fn package_items(text: &str) -> Option<Vec<PackageItem>> {
    let offset = if text.starts_with('\u{feff}') { 3 } else { 0 };
    let doc = roxmltree::Document::parse(&text[offset..]).ok()?;
    let shift = |r: Range<usize>| r.start + offset..r.end + offset;
    let mut out = Vec::new();
    for n in doc.descendants().filter(|n| n.is_element() && PACKAGE_ELEMENTS.contains(&n.tag_name().name())) {
        let Some(include) = n.attributes().find(|a| a.name() == "Include") else { continue };
        let id = include.value().trim();
        if id.is_empty() {
            continue;
        }
        let meta = |name: &str| -> Option<(String, Range<usize>, Range<usize>)> {
            if let Some(a) = n.attributes().find(|a| a.name() == name) {
                return Some((a.value().trim().to_string(), shift(a.range_value()), shift(a.range())));
            }
            let c = n.children().find(|c| c.has_tag_name(name))?;
            let t = c.first_child().filter(|t| t.is_text())?;
            let raw = t.range();
            let s = &text[offset..][raw.clone()];
            let lead = s.len() - s.trim_start().len();
            let value = raw.start + lead..raw.start + lead + s.trim().len();
            Some((s.trim().to_string(), shift(value), shift(c.range())))
        };
        let version = meta("Version");
        let over = meta("VersionOverride");
        let condition = n.attribute("Condition").or_else(|| n.parent_element().and_then(|g| g.attribute("Condition"))).map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
        let range = shift(n.range());
        out.push(PackageItem {
            element: n.tag_name().name().to_string(),
            id: id.to_string(),
            id_range: shift(include.range_value()),
            version: version.as_ref().map(|v| v.0.clone()),
            version_range: version.as_ref().map(|v| v.1.clone()),
            version_span: version.map(|v| v.2),
            version_override: over.as_ref().map(|v| v.0.clone()),
            override_range: over.map(|v| v.1),
            condition,
            line: text[..range.start].matches('\n').count(),
            range,
        });
    }
    Some(out)
}

/// Versions for completion: stable ones newest first, then prereleases newest first.
pub fn order_versions(mut versions: Vec<String>) -> Vec<String> {
    versions.sort_by(|a, b| a.contains('-').cmp(&b.contains('-')).then_with(|| compare_versions(b, a)));
    versions.dedup();
    versions
}

/// The newest stable version.
pub fn latest_stable(versions: &[String]) -> Option<String> {
    versions.iter().filter(|v| !v.contains('-')).max_by(|a, b| compare_versions(a, b)).cloned()
}

/// Whether `version` is a plain version (not `$(Property)`, a range or a wildcard).
pub fn is_plain_version(version: &str) -> bool {
    !version.is_empty() && version.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '+')
}

/// The `Include` value of the item a NuGet message is about: the longest package id of the file
/// that the message names as a whole word.
pub fn locate_message(text: &str, message: &str) -> Option<Range<usize>> {
    let items = package_items(text)?;
    let lower = message.to_ascii_lowercase();
    let word = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    let named = |id: &str| {
        let id = id.to_ascii_lowercase();
        lower.match_indices(&id).any(|(i, _)| {
            let before = lower[..i].chars().last();
            let mut after = lower[i + id.len()..].chars();
            // A dot ends the id only when no name part follows (`Logging.Abstractions`).
            let ends = match after.next() {
                None => true,
                Some('.') => !after.next().is_some_and(|c| c.is_ascii_alphanumeric()),
                c => !word(c),
            };
            !word(before) && before != Some('.') && ends
        })
    };
    items.iter().filter(|i| named(&i.id)).max_by_key(|i| i.id.len()).map(|i| i.id_range.clone())
}

/// Byte offset of a char offset.
pub fn byte_of(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map_or(text.len(), |(b, _)| b)
}

/// Char offset of a byte offset.
pub fn char_of(text: &str, byte: usize) -> usize {
    text[..floor_boundary(text, byte.min(text.len()))].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Context with the caret at `|`.
    fn ctx(s: &str) -> Option<XmlContext> {
        let caret = s.find('|').unwrap();
        let text = s.replacen('|', "", 1);
        context_at(&text, caret)
    }

    #[test]
    fn include_under_the_caret_with_partial_text() {
        let c = ctx(r#"<ItemGroup><PackageReference Include="Newt|" /></ItemGroup>"#).unwrap();
        assert_eq!((c.element.as_str(), c.attr, c.typed.as_str(), c.package_id.as_deref()), ("PackageReference", PackageAttr::Include, "Newt", Some("Newt")));
        // The caret in the middle: the value is the whole attribute.
        let s = r#"<PackageVersion Include="Ser|ilog" Version="3.0" />"#;
        let c = ctx(s).unwrap();
        let text = s.replace('|', "");
        assert_eq!(&text[c.value.clone()], "Serilog");
        assert_eq!(c.typed, "Ser");
        // Empty value right after typing the quotes.
        assert_eq!(ctx(r#"<GlobalPackageReference Include="|"/>"#).unwrap().typed, "");
    }

    #[test]
    fn version_attributes_know_their_package() {
        let c = ctx("<PackageReference\n    Include=\"Dapper\"\n    Version=\"2.|\" />").unwrap();
        assert_eq!((c.attr, c.typed.as_str(), c.package_id.as_deref()), (PackageAttr::Version, "2.", Some("Dapper")));
        let c = ctx(r#"<PackageReference Include="Dapper" VersionOverride="|"#).unwrap();
        assert_eq!((c.attr, c.package_id.as_deref()), (PackageAttr::VersionOverride, Some("Dapper")));
        // Unterminated value while typing, on an incomplete file.
        let c = ctx("<Project><ItemGroup><PackageReference Include=\"Xu|\n").unwrap();
        assert_eq!(c.typed, "Xu");
    }

    #[test]
    fn other_places_have_no_context() {
        assert!(ctx(r#"<ProjectReference Include="..\A|.csproj" />"#).is_none());
        assert!(ctx(r#"<PackageReference Include="A" />|"#).is_none());
        assert!(ctx(r#"<PackageReference Inc|lude="A" />"#).is_none());
        assert!(ctx(r#"<PackageReference Include="A" PrivateAssets="al|l" />"#).is_none());
        assert!(ctx(r#"<!-- <PackageReference --> text |"#).is_none());
    }

    #[test]
    fn items_have_ranges_for_ids_versions_and_conditions() {
        let text = "\u{feff}<Project>\n  <ItemGroup Condition=\"'$(X)' == 'a'\">\n    <PackageReference Include=\"A\" Version=\"1.0\" />\n    <PackageReference Include=\"B\">\n      <VersionOverride> 2.0 </VersionOverride>\n    </PackageReference>\n  </ItemGroup>\n</Project>";
        let items = package_items(text).unwrap();
        assert_eq!(items.len(), 2);
        let a = &items[0];
        assert_eq!((&text[a.id_range.clone()], a.version.as_deref(), a.line), ("A", Some("1.0"), 2));
        assert_eq!(&text[a.version_span.clone().unwrap()], r#"Version="1.0""#);
        assert_eq!(&text[a.version_range.clone().unwrap()], "1.0");
        assert_eq!(a.condition.as_deref(), Some("'$(X)' == 'a'"));
        let b = &items[1];
        assert_eq!((b.version_override.as_deref(), b.version.as_deref()), (Some("2.0"), None));
        assert_eq!(&text[b.override_range.clone().unwrap()], "2.0");
        assert!(package_items("<Project><ItemGroup>").is_none());
    }

    #[test]
    fn versions_are_newest_stable_first_then_prereleases() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let ordered = order_versions(v(&["1.0.0", "2.0.0-beta.2", "1.10.0", "1.9.0", "2.0.0-beta.10", "0.9.0"]));
        assert_eq!(ordered, v(&["1.10.0", "1.9.0", "1.0.0", "0.9.0", "2.0.0-beta.10", "2.0.0-beta.2"]));
        assert_eq!(latest_stable(&ordered).as_deref(), Some("1.10.0"));
        assert!(is_plain_version("13.0.3") && !is_plain_version("$(V)") && !is_plain_version("[1.0,2.0)"));
    }

    #[test]
    fn nuget_messages_point_at_their_package() {
        let text = "<Project>\n  <ItemGroup>\n    <PackageReference Include=\"Microsoft.Extensions.Logging\" />\n    <PackageReference Include=\"Microsoft.Extensions.Logging.Abstractions\" />\n    <PackageReference Include=\"Nope\" />\n  </ItemGroup>\n</Project>";
        let r = locate_message(text, "Unable to find package Microsoft.Extensions.Logging.Abstractions with version (>= 99.0.0)").unwrap();
        assert_eq!(&text[r], "Microsoft.Extensions.Logging.Abstractions");
        let r = locate_message(text, "Unable to find package Nope. No packages exist with this id in source(s): nuget.org").unwrap();
        assert_eq!(&text[r.clone()], "Nope");
        assert_eq!(text[..r.start].matches('\n').count(), 4);
        assert!(locate_message(text, "Some other problem").is_none());
    }

    #[test]
    fn package_files_are_recognized() {
        for f in ["A.csproj", "B.FSPROJ", "c.vbproj", "Directory.Packages.props", "directory.build.props", "Directory.Build.targets"] {
            assert!(is_package_file(Path::new(f)), "{f}");
        }
        assert!(!is_package_file(Path::new("Other.props")) && !is_package_file(Path::new("A.cs")));
        assert!(is_central_file(Path::new("/x/Directory.Packages.props")));
    }
}
