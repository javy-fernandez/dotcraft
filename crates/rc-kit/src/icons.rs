//! Icons: the IntelliJ "New UI" SVG set (see `assets/icons/NOTICE.md`), rasterized with resvg
//! at the exact physical pixel size they are shown at and painted on the pixel grid, so they
//! stay as sharp as in Rider at any zoom or display scale.

use crate::theme::Palette;
use egui::{Color32, ColorImage, Context, Id, Painter, Rect, TextureHandle, TextureOptions, pos2, vec2};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

#[allow(clippy::type_complexity)]
mod table {
    include!(concat!(env!("OUT_DIR"), "/icons_table.rs"));
}
use table::ICONS;

/// An icon of the set, named `dir/file` as in `assets/icons` (e.g. `nodes/class`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ico(pub &'static str);

impl Ico {
    fn sources(self) -> Option<(&'static [u8], Option<&'static [u8]>)> {
        ICONS.binary_search_by(|(n, _, _)| n.cmp(&self.0)).ok().map(|i| (ICONS[i].1, ICONS[i].2))
    }

    /// Whether the icon exists in the set.
    pub fn exists(self) -> bool {
        self.sources().is_some()
    }
}

/// Every icon name in the set (tests and the icon gallery).
pub fn all_names() -> impl Iterator<Item = &'static str> {
    ICONS.iter().map(|(n, _, _)| *n)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Variant {
    Normal,
    Disabled,
    /// Recoloured to one flat colour (icons on accent backgrounds).
    Tinted(Color32),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    name: &'static str,
    dark: bool,
    px: u32,
    variant: Variant,
}

#[derive(Clone, Default)]
struct Cache(Arc<Mutex<HashMap<Key, Option<TextureHandle>>>>);

fn texture(ctx: &Context, key: Key) -> Option<TextureHandle> {
    let cache = ctx.data_mut(|d| d.get_temp_mut_or_default::<Cache>(Id::new("dotcraft_icons")).clone());
    let mut map = cache.0.lock();
    if let Some(t) = map.get(&key) {
        return t.clone();
    }
    let t = rasterize(key).map(|img| ctx.load_texture(format!("icon:{}", key.name), img, TextureOptions::LINEAR));
    map.insert(key, t.clone());
    t
}

fn rasterize(key: Key) -> Option<ColorImage> {
    let (light, dark) = Ico(key.name).sources()?;
    let data = if key.dark { dark.unwrap_or(light) } else { light };
    let tree = resvg::usvg::Tree::from_data(data, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    // `px` is the pixel size of the 16-unit grid: smaller artwork (14×14 breakpoints, 7×10
    // badges) keeps its own size instead of being stretched, so it stays sharp.
    let scale = key.px.max(1) as f32 / 16.0;
    let (w, h) = ((size.width() * scale).round().max(1.0) as u32, (size.height() * scale).round().max(1.0) as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    let (sx, sy) = (w as f32 / size.width(), h as f32 / size.height());
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(sx, sy), &mut pixmap.as_mut());
    // tiny-skia pixels are premultiplied RGBA, like egui's `Color32`.
    let mut pixels: Vec<Color32> = pixmap
        .pixels()
        .iter()
        .map(|p| Color32::from_rgba_premultiplied(p.red(), p.green(), p.blue(), p.alpha()))
        .collect();
    match key.variant {
        Variant::Normal => {}
        Variant::Disabled => {
            // IntelliJ's disabled icons: desaturated and faded.
            for c in &mut pixels {
                let a = c.a() as f32 / 255.0;
                if a <= 0.0 {
                    continue;
                }
                let (r, g, b) = (c.r() as f32 / a, c.g() as f32 / a, c.b() as f32 / a);
                let gray = (0.299 * r + 0.587 * g + 0.114 * b).clamp(0.0, 255.0) as u8;
                let fade = if key.dark { 0.45 } else { 0.5 };
                *c = Color32::from_rgba_unmultiplied(gray, gray, gray, (c.a() as f32 * fade) as u8);
            }
        }
        Variant::Tinted(tint) => {
            for c in &mut pixels {
                let a = c.a();
                *c = Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), a);
            }
        }
    }
    Some(ColorImage::new([w as usize, h as usize], pixels))
}

/// Rect of `size` points centred in `rect`, snapped to whole physical pixels.
pub fn snapped(ctx: &Context, rect: Rect, size: f32) -> (Rect, u32) {
    let ppp = ctx.pixels_per_point();
    let px = (size * ppp).round().max(1.0);
    let c = rect.center();
    let min = pos2(((c.x - size / 2.0) * ppp).round() / ppp, ((c.y - size / 2.0) * ppp).round() / ppp);
    (Rect::from_min_size(min, vec2(px / ppp, px / ppp)), px as u32)
}

fn paint_variant(painter: &Painter, rect: Rect, ico: Ico, dark: bool, size: f32, variant: Variant) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let px = (size * ppp).round().max(1.0) as u32;
    if let Some(tex) = texture(ctx, Key { name: ico.0, dark, px, variant }) {
        // The texture is exactly as many pixels as it covers, on whole pixels.
        let [w, h] = tex.size();
        let (w, h) = (w as f32 / ppp, h as f32 / ppp);
        let c = rect.center();
        let min = pos2(((c.x - w / 2.0) * ppp).round() / ppp, ((c.y - h / 2.0) * ppp).round() / ppp);
        painter.image(tex.id(), Rect::from_min_size(min, vec2(w, h)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// Paints `ico` at its natural 16 pt size centred in `rect` (or the rect's size when smaller,
/// e.g. 12 pt tree chevrons). Disabled icons are greyed like IntelliJ's.
pub fn paint(painter: &Painter, rect: Rect, ico: Ico, p: &Palette, enabled: bool) {
    let side = rect.width().min(rect.height());
    // The set is drawn on a 16 pt grid: any smaller size would blur it.
    let size = if side >= 12.0 { 16.0 } else { side.round() };
    paint_variant(painter, rect, ico, p.dark, size, if enabled { Variant::Normal } else { Variant::Disabled });
}

/// Paints `ico` at exactly `size` points.
pub fn paint_sized(painter: &Painter, rect: Rect, ico: Ico, p: &Palette, enabled: bool, size: f32) {
    paint_variant(painter, rect, ico, p.dark, size, if enabled { Variant::Normal } else { Variant::Disabled });
}

/// Paints `ico` flattened to one colour (e.g. white on a blue primary button).
pub fn paint_tinted(painter: &Painter, rect: Rect, ico: Ico, dark: bool, size: f32, tint: Color32) {
    paint_variant(painter, rect, ico, dark, size, Variant::Tinted(tint));
}

/// Allocates `size`×`size` and shows the icon there.
pub fn image(ui: &mut egui::Ui, ico: Ico, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::hover());
    let p = crate::theme::pal(ui);
    paint_sized(ui.painter(), rect, ico, &p, ui.is_enabled(), size);
    resp
}

/// Named icons used across the IDE.
pub mod ico {
    use super::Ico;
    macro_rules! icons {
        ($($name:ident = $path:literal,)*) => {
            $(pub const $name: Ico = Ico($path);)*
            /// Every named icon, for the test that checks they exist.
            pub const ALL: &[Ico] = &[$($name,)*];
        };
    }
    icons! {
        // general
        ADD = "general/add",
        REMOVE = "general/remove",
        CLOSE = "general/close",
        CLOSE_SMALL = "general/closeSmall",
        CLOSE_SMALL_HOVERED = "general/closeSmallHovered",
        CHEVRON_DOWN = "general/chevronDown",
        CHEVRON_UP = "general/chevronUp",
        CHEVRON_LEFT = "general/chevronLeft",
        CHEVRON_RIGHT = "general/chevronRight",
        COLLAPSE_ALL = "general/collapseAll",
        EXPAND_ALL = "general/expandAll",
        COPY = "general/copy",
        CUT = "general/cut",
        PASTE = "general/paste",
        DELETE = "general/delete",
        EDIT = "general/edit",
        FILTER = "general/filter",
        HIDE = "general/hide",
        HISTORY = "general/history",
        LOCATE = "general/locate",
        MORE = "general/moreHorizontal",
        MORE_VERT = "general/moreVertical",
        REFRESH = "general/refresh",
        SEARCH = "general/search",
        SETTINGS = "general/settings",
        SHOW = "general/show",
        SORT = "general/sortBy",
        SPLIT_RIGHT = "general/splitVertically",
        SPLIT_DOWN = "general/splitHorizontally",
        UNDO = "general/undo",
        REDO = "general/redo",
        USER = "general/user",
        VCS = "general/vcs",
        HELP = "general/help",
        SAVE = "general/save",
        SOFT_WRAP = "general/softWrap",
        OPEN = "general/open",
        AUTOSCROLL_FROM_SOURCE = "general/autoscrollFromSource",
        CHECKMARK = "general/greenCheckmark",
        PIN = "general/pin",
        MENU = "general/menu",
        EXPORT = "general/export",
        DOWNLOAD = "general/download",
        IMPORT = "general/import",
        UP = "general/up",
        DOWN = "general/down",
        DIALOG_ERROR = "general/errorDialog",
        DIALOG_WARNING = "general/warningDialog",
        DIALOG_INFO = "general/informationDialog",
        DIALOG_QUESTION = "general/questionDialog",
        RUN_ANYTHING = "general/runAnything",
        // actions
        NEW_FILE = "actions/addFile",
        NEW_FOLDER = "actions/newFolder",
        REFORMAT = "actions/reformatCode",
        REPLACE = "actions/replace",
        FIND_NEXT = "actions/findForward",
        FIND_PREV = "actions/findBackward",
        IN_SELECTION = "actions/inSelection",
        PREVIEW = "actions/preview",
        RUN_ALL = "actions/runAll",
        // inline (search field toggles)
        MATCH_CASE = "inline/matchCase",
        REGEX = "inline/regex",
        WHOLE_WORDS = "inline/exactWords",
        PRESERVE_CASE = "inline/preserveCase",
        SEARCH_HISTORY = "inline/searchHistory",
        INLINE_CLOSE = "inline/inlineClose",
        // tool windows
        TW_PROJECT = "toolwindows/project",
        TW_BUILD = "toolwindows/build",
        TW_RUN = "toolwindows/run",
        TW_DEBUG = "toolwindows/debug",
        TW_COMMIT = "toolwindows/commit",
        TW_VCS = "toolwindows/vcs",
        TW_PROBLEMS = "toolwindows/problems",
        TW_STRUCTURE = "toolwindows/structure",
        TW_NOTIFICATIONS = "toolwindows/notifications",
        TW_FIND = "toolwindows/find",
        TW_SERVICES = "toolwindows/services",
        TW_TODO = "toolwindows/todo",
        TW_BOOKMARKS = "toolwindows/bookmarks",
        TW_DEPENDENCIES = "toolwindows/dependencies",
        TW_MESSAGES = "toolwindows/messages",
        TW_OVERFLOW = "toolwindows/toolWindowOverflow",
        // run & debug
        RUN = "run/run",
        DEBUG = "run/debug",
        STOP = "run/stop",
        RERUN = "run/rerun",
        RESTART = "run/restart",
        RESUME = "run/resume",
        PAUSE = "run/pause",
        STEP_OVER = "run/stepOver",
        STEP_INTO = "run/stepInto",
        STEP_OUT = "run/stepOut",
        RUN_TO_CURSOR = "run/runToCursor",
        EVALUATE = "run/evaluateExpression",
        SHOW_EXECUTION_POINT = "run/showCurrentFrame",
        EXECUTION_POINT = "debugger/nextStatement",
        MUTE_BREAKPOINTS = "run/muteBreakpoints",
        VIEW_BREAKPOINTS = "run/viewBreakpoints",
        RESTART_FAILED = "run/restartFailedTests",
        KILL = "run/killProcess",
        SHOW_PASSED = "run/showPassed",
        SHOW_IGNORED = "run/showIgnored",
        ATTACH = "run/attachToProcess",
        TEST_PASSED = "run/testPassed",
        TEST_FAILED = "run/testFailed",
        TEST_ERROR = "run/testError",
        TEST_IGNORED = "run/testIgnored",
        TEST_NOT_RUN = "run/testNotRunYet",
        TEST_SKIPPED = "run/testSkipped",
        TEST_PAUSED = "run/testPaused",
        TEST_TERMINATED = "run/testTerminated",
        TEST_UNKNOWN = "run/testUnknown",
        BUILD = "build/build",
        REBUILD = "build/rebuild",
        // breakpoints
        BREAKPOINT = "breakpoints/breakpoint",
        BREAKPOINT_VALID = "breakpoints/breakpointValid",
        BREAKPOINT_INVALID = "breakpoints/breakpointInvalid",
        BREAKPOINT_DISABLED = "breakpoints/breakpointDisabled",
        BREAKPOINT_MUTED = "breakpoints/breakpointMuted",
        BREAKPOINT_EXCEPTION = "breakpoints/breakpointException",
        BREAKPOINT_DEPENDENT = "breakpoints/breakpointDependent",
        QUESTION_BADGE = "breakpoints/questionBadge",
        // debugger
        WATCH = "debugger/watch",
        ADD_TO_WATCH = "debugger/addToWatch",
        FRAME = "debugger/frame",
        THREADS = "debugger/threads",
        THREAD_CURRENT = "debugger/threadCurrent",
        THREAD_AT_BREAKPOINT = "debugger/threadAtBreakpoint",
        VAR_OBJECT = "debugger/dbObject",
        VAR_ARRAY = "debugger/dbArray",
        VAR_PRIMITIVE = "debugger/dbPrimitive",
        VAR_SPECIAL = "debugger/specialVar",
        EVALUATION_RESULT = "debugger/evaluationResult",
        VARIABLES = "debugger/variablesTab",
        // gutter
        GUTTER_RUN = "gutter/run",
        GUTTER_RERUN = "gutter/rerun",
        GUTTER_RUN_SUCCESS = "gutter/runSuccess",
        GUTTER_RUN_ERROR = "gutter/runError",
        GUTTER_RUN_FAILED = "gutter/runFailed",
        GUTTER_IMPLEMENTED = "gutter/implementedMethod",
        GUTTER_IMPLEMENTING = "gutter/implementingMethod",
        GUTTER_OVERRIDDEN = "gutter/overridenMethod",
        GUTTER_OVERRIDING = "gutter/overridingMethod",
        GUTTER_BOOKMARK = "gutter/bookmark",
        GUTTER_FOLD = "gutter/fold",
        GUTTER_UNFOLD = "gutter/unfold",
        GUTTER_FOLD_BOTTOM = "gutter/foldBottom",
        // code insight
        BULB = "codeInsight/intentionBulb",
        BULB_QUICKFIX = "codeInsight/quickfixBulb",
        BULB_GREY = "codeInsight/intentionBulbGrey",
        // status
        ERROR = "status/error",
        WARNING = "status/warning",
        INFO = "status/info",
        SUCCESS = "status/success",
        INSPECTIONS_OK = "inspections/inspectionsOK",
        INSPECTIONS_ERROR = "inspections/inspectionsError",
        INSPECTIONS_WARNING = "inspections/inspectionsWarning",
        INSPECTIONS_TYPO = "inspections/inspectionsTypos",
        // files
        FILE_CS = "fileTypes/Csharp",
        FILE_XML = "fileTypes/xml",
        FILE_JSON = "fileTypes/json",
        FILE_MARKDOWN = "fileTypes/markdown",
        FILE_TEXT = "fileTypes/text",
        FILE_CONFIG = "fileTypes/config",
        FILE_UNKNOWN = "fileTypes/unknown",
        FILE_GITIGNORE = "fileTypes/gitignore",
        FILE_EDITORCONFIG = "fileTypes/editorConfig",
        FILE_IMAGE = "fileTypes/image",
        FILE_ARCHIVE = "fileTypes/archive",
        FILE_CSS = "fileTypes/css",
        FILE_HTML = "fileTypes/html",
        FILE_JS = "fileTypes/javaScript",
        FILE_TS = "fileTypes/typeScript",
        FILE_YAML = "fileTypes/yaml",
        FILE_SHELL = "fileTypes/shell",
        FILE_SQL = "fileTypes/sql",
        FILE_PROPERTIES = "fileTypes/properties",
        FILE_PATCH = "fileTypes/patch",
        FILE_CSV = "fileTypes/csv",
        FILE_BINARY = "fileTypes/binaryData",
        FILE_WINDOWS = "fileTypes/microsoftWindows",
        // tree nodes
        FOLDER = "nodes/folder",
        SOLUTION = "nodes/ideaProject",
        PROJECT = "nodes/module",
        PROJECT_GROUP = "nodes/moduleGroup",
        LIBRARY = "nodes/library",
        PACKAGE = "nodes/package",
        SOURCE_ROOT = "nodes/sourceRoot",
        TEST_ROOT = "nodes/testRoot",
        EXCLUDED = "nodes/excludeRoot",
        PLUGIN = "nodes/plugin",
        SERVICES = "nodes/services",
        // symbols
        CLASS = "nodes/class",
        CLASS_ABSTRACT = "nodes/classAbstract",
        INTERFACE = "nodes/interface",
        ENUM = "nodes/enum",
        RECORD = "nodes/record",
        STRUCT = "nodes/static",
        FIELD = "nodes/field",
        PROPERTY = "nodes/property",
        METHOD = "nodes/method",
        METHOD_ABSTRACT = "nodes/methodAbstract",
        FUNCTION = "nodes/function",
        CONSTANT = "nodes/constant",
        CONSTRUCTOR = "nodes/constructor",
        VARIABLE = "nodes/variable",
        PARAMETER = "nodes/parameter",
        LAMBDA = "nodes/lambda",
        NAMESPACE = "nodes/package",
        ANNOTATION = "nodes/annotation",
        EXCEPTION = "nodes/exception",
        TYPE = "nodes/type",
        TAG = "nodes/tag",
        TEMPLATE = "nodes/template",
        KEYWORD = "nodes/word",
        ALIAS = "nodes/alias",
        TEST = "nodes/test",
        TEST_GROUP = "nodes/testGroup",
        EVENT = "nodes/gvariable",
        // vcs
        COMMIT = "vcs/commit",
        PUSH = "vcs/push",
        UPDATE = "vcs/update",
        FETCH = "vcs/fetch",
        MERGE = "vcs/merge",
        REVERT = "vcs/revert",
        DIFF = "vcs/diff",
        CHANGELIST = "vcs/changelist",
        SHELVE = "vcs/shelve",
        UNVERSIONED = "vcs/showUnversionedFiles",
        ARROW_LEFT = "vcs/arrowLeft",
        ARROW_RIGHT = "vcs/arrowRight",
        DIFF_SIDE_BY_SIDE = "diff/sideBySide",
        DIFF_UNIFIED = "diff/unified",
        // windows caption
        WIN_CLOSE = "windows/close",
        WIN_MAXIMIZE = "windows/maximize",
        WIN_MINIMIZE = "windows/minimize",
        WIN_RESTORE = "windows/restore",
        // run configurations
        CONFIG_APP = "runConfigurations/application",
        CONFIG_WEB = "runConfigurations/webApp",
        CONFIG_TESTS = "runConfigurations/junit",
        CONFIG_COMPOUND = "runConfigurations/compound",
        // misc
        EXTERNAL_LINK = "ide/externalLink",
        HIERARCHY = "hierarchy/classHierarchy",
        SORT_ALPHA = "objectBrowser/sortAlphabetically",
        SHOW_MEMBERS = "objectBrowser/showMembers",
        BOOKMARK = "bookmarks/bookmark",
        JSON_OBJECT = "json/object",
        JSON_ARRAY = "json/array",
    }
}

/// Icon for a file in the explorer and the editor tabs.
pub fn for_file(name: &str) -> Ico {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
    match ext {
        _ if lower == ".gitignore" || lower == ".gitattributes" => ico::FILE_GITIGNORE,
        _ if lower == ".editorconfig" => ico::FILE_EDITORCONFIG,
        "cs" | "csx" => ico::FILE_CS,
        "sln" | "slnx" | "slnf" => ico::SOLUTION,
        "csproj" | "fsproj" | "vbproj" | "proj" => ico::PROJECT,
        "props" | "targets" | "xml" | "resx" | "xaml" | "axaml" | "nuspec" | "runsettings" | "xsd" | "manifest" => ico::FILE_XML,
        "json" | "jsonc" => ico::FILE_JSON,
        "config" => ico::FILE_CONFIG,
        "md" | "markdown" => ico::FILE_MARKDOWN,
        "txt" | "log" => ico::FILE_TEXT,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico" | "svg" | "webp" => ico::FILE_IMAGE,
        "zip" | "nupkg" | "snupkg" | "gz" | "tar" | "7z" => ico::FILE_ARCHIVE,
        "css" | "scss" | "less" => ico::FILE_CSS,
        "html" | "htm" | "cshtml" | "razor" => ico::FILE_HTML,
        "js" | "mjs" | "cjs" => ico::FILE_JS,
        "ts" | "tsx" => ico::FILE_TS,
        "yml" | "yaml" => ico::FILE_YAML,
        "sh" | "bash" | "zsh" | "ps1" | "cmd" | "bat" => ico::FILE_SHELL,
        "sql" => ico::FILE_SQL,
        "properties" | "ini" | "env" => ico::FILE_PROPERTIES,
        "patch" | "diff" => ico::FILE_PATCH,
        "csv" | "tsv" => ico::FILE_CSV,
        "dll" | "exe" | "pdb" | "so" | "dylib" | "bin" => ico::FILE_BINARY,
        _ => ico::FILE_TEXT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_icon_exists_and_rasterizes_in_both_themes() {
        for icon in ico::ALL {
            assert!(icon.exists(), "missing icon {}", icon.0);
            for dark in [false, true] {
                for variant in [Variant::Normal, Variant::Disabled] {
                    let img = rasterize(Key { name: icon.0, dark, px: 32, variant }).unwrap_or_else(|| panic!("{} fails to render", icon.0));
                    assert!(img.size[0] >= 8 && img.size[1] >= 8, "{} is {:?}", icon.0, img.size);
                    assert!(img.pixels.iter().any(|p| p.a() > 0), "{} renders empty", icon.0);
                }
            }
        }
    }

    #[test]
    fn icons_snap_to_the_pixel_grid() {
        let ctx = Context::default();
        ctx.set_pixels_per_point(2.0);
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        let (r, px) = snapped(&ctx, Rect::from_min_size(pos2(10.3, 4.1), vec2(26.0, 26.0)), 16.0);
        assert_eq!(px, 32);
        assert_eq!((r.min.x * 2.0).fract(), 0.0);
        assert_eq!((r.min.y * 2.0).fract(), 0.0);
        assert_eq!(r.width(), 16.0);
    }
}
