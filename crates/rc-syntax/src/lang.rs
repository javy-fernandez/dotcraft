//! Supported languages and their grammars/queries (built once per process).

use std::path::Path;
use std::sync::OnceLock;
use tree_sitter::{Language, Query};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    CSharp,
    Xml,
    Json,
    Markdown,
    /// Razor markup (`.razor`, `.cshtml`): C# and HTML.
    Razor,
    /// Visual Studio solution files (`.sln`), highlighted by a small lexer (no grammar).
    Sln,
    Plain,
}

impl Lang {
    pub fn from_path(path: &Path) -> Lang {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "cs" | "csx" => Lang::CSharp,
            "csproj" | "props" | "targets" | "xml" | "config" | "resx" | "xaml" | "axaml" | "nuspec" | "slnx" | "fsproj"
            | "vbproj" | "runsettings" | "xsd" | "svg" => Lang::Xml,
            "json" | "jsonc" => Lang::Json,
            "md" | "markdown" => Lang::Markdown,
            // Plain HTML reads well as Razor without `@` (its markup lexer colours the tags).
            "razor" | "cshtml" | "html" | "htm" => Lang::Razor,
            "sln" => Lang::Sln,
            _ => {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.eq_ignore_ascii_case("Directory.Packages.props") { Lang::Xml } else { Lang::Plain }
            }
        }
    }

    /// Prefix used by "Comment with line comment".
    pub fn line_comment(self) -> Option<&'static str> {
        match self {
            Lang::CSharp | Lang::Json => Some("//"),
            Lang::Sln => Some("#"),
            _ => None,
        }
    }

    /// Block comment delimiters (XML has no line comments).
    pub fn block_comment(self) -> Option<(&'static str, &'static str)> {
        match self {
            Lang::CSharp => Some(("/*", "*/")),
            Lang::Xml | Lang::Markdown => Some(("<!--", "-->")),
            Lang::Razor => Some(("@*", "*@")),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::CSharp => "C#",
            Lang::Xml => "XML",
            Lang::Json => "JSON",
            Lang::Markdown => "Markdown",
            Lang::Razor => "Razor",
            Lang::Sln => "Solution",
            Lang::Plain => "Plain text",
        }
    }

    pub(crate) fn language(self) -> Option<Language> {
        Some(match self {
            Lang::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
            Lang::Xml => tree_sitter_xml::LANGUAGE_XML.into(),
            Lang::Json => tree_sitter_json::LANGUAGE.into(),
            Lang::Markdown => tree_sitter_md::LANGUAGE.into(),
            Lang::Razor => tree_sitter_razor_modern::LANGUAGE.into(),
            Lang::Sln | Lang::Plain => return None,
        })
    }

    /// The highlight query, compiled once.
    pub(crate) fn query(self) -> Option<&'static Query> {
        static CS: OnceLock<Query> = OnceLock::new();
        static XML: OnceLock<Query> = OnceLock::new();
        static JSON: OnceLock<Query> = OnceLock::new();
        static MD: OnceLock<Query> = OnceLock::new();
        static RAZOR: OnceLock<Query> = OnceLock::new();
        // Razor's C# uses the C# grammar's node kinds: the C# patterns apply, Razor's come after.
        static RAZOR_SOURCE: OnceLock<String> = OnceLock::new();
        let (cell, source) = match self {
            Lang::CSharp => (&CS, include_str!("../queries/csharp.scm")),
            Lang::Xml => (&XML, include_str!("../queries/xml.scm")),
            Lang::Json => (&JSON, include_str!("../queries/json.scm")),
            Lang::Markdown => (&MD, include_str!("../queries/markdown.scm")),
            Lang::Razor => (&RAZOR, RAZOR_SOURCE.get_or_init(|| format!("{}\n{}", include_str!("../queries/csharp.scm"), include_str!("../queries/razor.scm"))).as_str()),
            Lang::Sln | Lang::Plain => return None,
        };
        let language = self.language()?;
        Some(cell.get_or_init(|| Query::new(&language, source).unwrap_or_else(|e| panic!("bad {self:?} query: {e}"))))
    }
}
