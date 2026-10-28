//! "Add" items (Rider's Add menu in the explorer): C# types, ASP.NET controllers, Razor
//! components, pages and views, test classes and plain files, with their namespace.

use rc_kit::{Ico, ico};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Class,
    Interface,
    Record,
    Struct,
    Enum,
    ApiController,
    MvcController,
    RazorComponent,
    RazorPage,
    MvcView,
    TestClass,
    File,
    Json,
    Text,
    Folder,
}

impl ItemKind {
    /// The Add menu, in order (`None` marks a separator).
    pub const MENU: &[Option<ItemKind>] = &[
        Some(ItemKind::Class),
        Some(ItemKind::Interface),
        Some(ItemKind::Record),
        Some(ItemKind::Struct),
        Some(ItemKind::Enum),
        None,
        Some(ItemKind::ApiController),
        Some(ItemKind::MvcController),
        Some(ItemKind::RazorComponent),
        Some(ItemKind::RazorPage),
        Some(ItemKind::MvcView),
        Some(ItemKind::TestClass),
        None,
        Some(ItemKind::File),
        Some(ItemKind::Json),
        Some(ItemKind::Text),
        Some(ItemKind::Folder),
    ];

    /// English menu text for `tr`.
    pub fn title(self) -> &'static str {
        match self {
            ItemKind::Class => "Class",
            ItemKind::Interface => "Interface",
            ItemKind::Record => "Record",
            ItemKind::Struct => "Struct",
            ItemKind::Enum => "Enum",
            ItemKind::ApiController => "API Controller",
            ItemKind::MvcController => "MVC Controller",
            ItemKind::RazorComponent => "Razor Component",
            ItemKind::RazorPage => "Razor Page",
            ItemKind::MvcView => "MVC View",
            ItemKind::TestClass => "Unit Test Class",
            ItemKind::File => "File",
            ItemKind::Json => "JSON File",
            ItemKind::Text => "Text File",
            ItemKind::Folder => "Directory",
        }
    }

    pub fn icon(self) -> Ico {
        match self {
            ItemKind::Class => ico::CLASS,
            ItemKind::Interface => ico::INTERFACE,
            ItemKind::Record => ico::RECORD,
            ItemKind::Struct => ico::STRUCT,
            ItemKind::Enum => ico::ENUM,
            ItemKind::ApiController | ItemKind::MvcController => ico::CONFIG_WEB,
            ItemKind::RazorComponent | ItemKind::RazorPage | ItemKind::MvcView => ico::FILE_HTML,
            ItemKind::TestClass => ico::RUN_ALL,
            ItemKind::File => ico::NEW_FILE,
            ItemKind::Json => ico::FILE_JSON,
            ItemKind::Text => ico::FILE_TEXT,
            ItemKind::Folder => ico::FOLDER,
        }
    }

    pub fn is_folder(self) -> bool {
        self == ItemKind::Folder
    }

    /// Files named as typed, without a namespace (File, JSON, Text).
    pub fn is_plain(self) -> bool {
        matches!(self, ItemKind::File | ItemKind::Json | ItemKind::Text)
    }

    fn extension(self) -> &'static str {
        match self {
            ItemKind::RazorComponent => "razor",
            ItemKind::RazorPage | ItemKind::MvcView => "cshtml",
            ItemKind::Json => "json",
            ItemKind::Text => "txt",
            ItemKind::File | ItemKind::Folder => "",
            _ => "cs",
        }
    }

    /// The name typed, cleaned up: no extension, and `Controller`/`Tests` suffixes added where
    /// the convention wants them.
    pub fn type_name(self, typed: &str) -> String {
        let ext = format!(".{}", self.extension());
        let base = typed.trim().trim_end_matches(ext.as_str()).trim_end_matches(".cs").to_string();
        match self {
            ItemKind::ApiController | ItemKind::MvcController if !base.ends_with("Controller") => format!("{base}Controller"),
            _ => base,
        }
    }

    /// Files to create in `dir` for `typed`: (path, content with `$0` where the caret goes).
    /// The first one is opened.
    pub fn files(self, dir: &Path, typed: &str) -> Vec<(PathBuf, String)> {
        let name = self.type_name(typed);
        if self.is_folder() || self == ItemKind::File {
            return vec![(dir.join(typed.trim()), if self.is_folder() { String::new() } else { "$0".into() })];
        }
        // Plain files keep the extension typed.
        if matches!(self, ItemKind::Json | ItemKind::Text) {
            let file = if typed.contains('.') { typed.trim().to_string() } else { format!("{name}.{}", self.extension()) };
            let content = if self == ItemKind::Json { "{\n  $0\n}\n".to_string() } else { "$0".to_string() };
            return vec![(dir.join(file), content)];
        }
        let ns = namespace_for(dir);
        let path = dir.join(format!("{name}.{}", self.extension()));
        let body = |kind: &str, extra: &str| format!("namespace {ns};\n\npublic {kind} {name}{extra}\n{{\n    $0\n}}\n");
        match self {
            ItemKind::Class => vec![(path, body("class", ""))],
            ItemKind::Interface => vec![(path, body("interface", ""))],
            ItemKind::Record => vec![(path, body("record", ""))],
            ItemKind::Struct => vec![(path, body("struct", ""))],
            ItemKind::Enum => vec![(path, body("enum", ""))],
            ItemKind::ApiController => vec![(
                path,
                format!(
                    "using Microsoft.AspNetCore.Mvc;\n\nnamespace {ns};\n\n[ApiController]\n[Route(\"api/[controller]\")]\npublic class {name} : ControllerBase\n{{\n    [HttpGet]\n    public IActionResult Get()\n    {{\n        $0return Ok();\n    }}\n}}\n"
                ),
            )],
            ItemKind::MvcController => vec![(
                path,
                format!("using Microsoft.AspNetCore.Mvc;\n\nnamespace {ns};\n\npublic class {name} : Controller\n{{\n    public IActionResult Index()\n    {{\n        $0return View();\n    }}\n}}\n"),
            )],
            ItemKind::RazorComponent => vec![(path, format!("<h3>{name}</h3>\n\n@code {{\n    $0\n}}\n"))],
            ItemKind::MvcView => vec![(path, format!("@{{\n    ViewData[\"Title\"] = \"{name}\";\n}}\n\n<h1>{name}</h1>\n$0\n"))],
            ItemKind::RazorPage => {
                let model = format!("{name}Model");
                let page = format!("@page\n@model {ns}.{model}\n@{{\n    ViewData[\"Title\"] = \"{name}\";\n}}\n\n<h1>{name}</h1>\n$0\n");
                let code = format!(
                    "using Microsoft.AspNetCore.Mvc.RazorPages;\n\nnamespace {ns};\n\npublic class {model} : PageModel\n{{\n    public void OnGet()\n    {{\n    }}\n}}\n"
                );
                vec![(path, page), (dir.join(format!("{name}.cshtml.cs")), code)]
            }
            ItemKind::TestClass => {
                let framework = test_framework(dir);
                let (using, class_attr, method_attr) = match framework {
                    "nunit" => ("using NUnit.Framework;\n\n", "[TestFixture]\n", "[Test]"),
                    "mstest" => ("using Microsoft.VisualStudio.TestTools.UnitTesting;\n\n", "[TestClass]\n", "[TestMethod]"),
                    _ => ("using Xunit;\n\n", "", "[Fact]"),
                };
                vec![(path, format!("{using}namespace {ns};\n\n{class_attr}public class {name}\n{{\n    {method_attr}\n    public void Test1()\n    {{\n        $0\n    }}\n}}\n"))]
            }
            ItemKind::File | ItemKind::Json | ItemKind::Text | ItemKind::Folder => unreachable!("handled above"),
        }
    }
}

/// The project file that owns `dir` (the nearest `.csproj` above it).
fn project_of(dir: &Path) -> Option<PathBuf> {
    let mut d = Some(dir);
    while let Some(cur) = d {
        if let Some(p) = std::fs::read_dir(cur).ok()?.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "csproj")) {
            return Some(p);
        }
        d = cur.parent();
    }
    None
}

/// `RootNamespace` (or the project name) plus the folders below the project, like Rider.
pub fn namespace_for(dir: &Path) -> String {
    let clean = |s: &str| s.replace([' ', '-'], "_");
    let Some(project) = project_of(dir) else {
        return dir.file_name().map(|n| clean(&n.to_string_lossy())).unwrap_or_else(|| "App".into());
    };
    let text = std::fs::read_to_string(&project).unwrap_or_default();
    let root = text
        .split("<RootNamespace>")
        .nth(1)
        .and_then(|r| r.split("</RootNamespace>").next())
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.contains('$'))
        .map(str::to_string)
        .unwrap_or_else(|| clean(&project.file_stem().unwrap_or_default().to_string_lossy()));
    let rel = dir.strip_prefix(project.parent().unwrap_or(dir)).unwrap_or(Path::new(""));
    std::iter::once(root).chain(rel.components().map(|c| clean(&c.as_os_str().to_string_lossy()))).collect::<Vec<_>>().join(".")
}

/// The test framework a project references (`xunit` by default).
fn test_framework(dir: &Path) -> &'static str {
    let text = project_of(dir).and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default().to_ascii_lowercase();
    if text.contains("\"nunit\"") || text.contains("include=\"nunit") {
        "nunit"
    } else if text.contains("mstest") {
        "mstest"
    } else {
        "xunit"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_controllers_and_pages() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("Shop.Web");
        std::fs::create_dir_all(proj.join("Controllers/Admin")).unwrap();
        std::fs::write(proj.join("Shop.Web.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk.Web\"><PropertyGroup><RootNamespace>Shop.Site</RootNamespace></PropertyGroup></Project>").unwrap();
        let admin = proj.join("Controllers/Admin");
        assert_eq!(namespace_for(&admin), "Shop.Site.Controllers.Admin");
        let files = ItemKind::ApiController.files(&admin, "Orders");
        assert_eq!(files[0].0, admin.join("OrdersController.cs"));
        assert!(files[0].1.contains("namespace Shop.Site.Controllers.Admin;") && files[0].1.contains("public class OrdersController : ControllerBase"));
        let page = ItemKind::RazorPage.files(&proj, "Privacy");
        assert_eq!(page.len(), 2);
        assert!(page[0].1.contains("@model Shop.Site.PrivacyModel") && page[1].1.contains("class PrivacyModel : PageModel"));
        assert_eq!(ItemKind::Json.files(&proj, "appsettings.Local.json")[0].0, proj.join("appsettings.Local.json"));
        assert_eq!(ItemKind::Interface.files(&proj, "IClock.cs")[0].0, proj.join("IClock.cs"));
    }

    #[test]
    fn plain_files_keep_the_name_typed() {
        let dir = Path::new("/w");
        assert_eq!(ItemKind::File.files(dir, " Dockerfile ")[0], (dir.join("Dockerfile"), "$0".to_string()));
        assert_eq!(ItemKind::File.files(dir, "docs/notes.md")[0].0, dir.join("docs/notes.md"));
        assert_eq!(ItemKind::Text.files(dir, "notes")[0].0, dir.join("notes.txt"));
    }
}
