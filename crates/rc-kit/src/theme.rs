//! "Islands" visual theme (Light / Dark) taken from the JetBrains 2026.2 themes
//! (`ManyIslandsLight.theme.json`, `ManyIslandsDark.theme.json` and the "Light" / "Dark"
//! editor schemes), Inter + JetBrains Mono fonts and common painting helpers.

use crate::icons::{self, Ico, ico};
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Rect, Response, Sense, Shadow,
    Stroke, TextStyle, Ui, Vec2, vec2,
};
use std::sync::Arc;

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// `#RRGGBBAA` as in IntelliJ themes (non-premultiplied alpha).
const fn hexa(rgba: u32) -> Color32 {
    let a = rgba & 0xFF;
    Color32::from_rgba_premultiplied(
        (((rgba >> 24) & 0xFF) * a / 255) as u8,
        (((rgba >> 16) & 0xFF) * a / 255) as u8,
        (((rgba >> 8) & 0xFF) * a / 255) as u8,
        a as u8,
    )
}

/// Palette used by the custom components. Each field names the source theme key.
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// MainWindow.background / MainToolbar / StatusBar / ToolWindow.Stripe (main-window-bg).
    pub main_bg: Color32,
    /// Island.borderColor / ToolWindow.background / editor-bg.
    pub island_bg: Color32,
    /// editor-border (subtle island border and inner separators).
    pub island_border: Color32,
    /// text-default.
    pub text: Color32,
    /// text-muted (StatusBar.Widget.foreground, Breadcrumbs).
    pub text_muted: Color32,
    /// text-secondary (infoForeground: counters, column types).
    pub muted: Color32,
    /// text-disabled.
    pub disabled: Color32,
    /// Line icon color ("new UI" gray).
    pub icon: Color32,
    /// toolbar-bg-hovered.
    pub hover: Color32,
    /// toolbar-bg-pressed.
    pub pressed: Color32,
    /// Tree/Table.selectionBackground (selection-bg-active).
    pub selection: Color32,
    /// selectionInactiveBackground.
    pub selection_inactive: Color32,
    /// List/Tree.hoverBackground.
    pub row_hover: Color32,
    /// EditorTabs.underlinedTabBackground / underlinedBorderColor.
    pub tab_selected_bg: Color32,
    pub tab_selected_border: Color32,
    /// Component.borderColor (control-border).
    pub control_border: Color32,
    /// TextField.background (control-bg).
    pub control_bg: Color32,
    /// Button.default.startBackground (control-brand-bg).
    pub accent: Color32,
    /// Table.gridColor.
    pub grid: Color32,
    /// Table header background.
    pub header_bg: Color32,
    /// Popup.background.
    pub popup_bg: Color32,
    /// Popup.borderColor.
    pub popup_border: Color32,
    /// Background of the active button in the tool window stripe.
    pub stripe_selected: Color32,
    /// List.Tag (tree badges).
    pub badge_bg: Color32,
    pub badge_fg: Color32,
    // --- editor scheme ---
    pub editor_bg: Color32,
    pub gutter_fg: Color32,
    pub gutter_fg_caret: Color32,
    pub caret_row: Color32,
    pub editor_selection: Color32,
    // --- states ---
    pub error: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub run: Color32,
    pub stop: Color32,
}

pub const LIGHT: Palette = Palette {
    dark: false,
    main_bg: hex(0xE9EAEE),
    island_bg: hex(0xFFFFFF),
    island_border: hex(0xE9EAEE),
    text: hex(0x000000),
    text_muted: hex(0x5F6269),
    muted: hex(0x73767C),
    disabled: hex(0x9FA2A8),
    icon: hex(0x6C707E),
    hover: hexa(0x00000012),
    pressed: hexa(0x00000020),
    selection: hex(0xD0DFFE),
    selection_inactive: hex(0xE9EAEE),
    row_hover: hexa(0x00000008),
    tab_selected_bg: hex(0xE3EBFE),
    tab_selected_border: hex(0xA7C5FF),
    control_border: hex(0xD1D3D9),
    control_bg: hex(0xFFFFFF),
    accent: hex(0x3871E1),
    grid: hex(0xE9EAEE),
    header_bg: hex(0xF7F8F9),
    popup_bg: hex(0xFFFFFF),
    popup_border: hex(0xD1D3D9),
    stripe_selected: hex(0xDDDFE4),
    badge_bg: hex(0xEBECF0),
    badge_fg: hex(0x494B57),
    editor_bg: hex(0xFFFFFF),
    gutter_fg: hex(0x888888),
    gutter_fg_caret: hex(0x383838),
    caret_row: hex(0xF4F8F8),
    editor_selection: hex(0xB8D9FF),
    error: hex(0xC54E58),
    ok: hex(0x338555),
    warn: hex(0xA56906),
    run: hex(0x369650),
    stop: hex(0xDB5C5C),
};

pub const DARK: Palette = Palette {
    dark: true,
    main_bg: hex(0x26282C),
    island_bg: hex(0x191A1C),
    island_border: hex(0x26282C),
    text: hex(0xD1D3D9),
    text_muted: hex(0x9FA2A8),
    muted: hex(0x73767C),
    disabled: hex(0x4C4F56),
    icon: hex(0xA4A7AE),
    hover: hexa(0xFFFFFF17),
    pressed: hexa(0xFFFFFF21),
    selection: hex(0x2A4371),
    selection_inactive: hex(0x33353B),
    row_hover: hexa(0xFFFFFF10),
    tab_selected_bg: hex(0x233558),
    tab_selected_border: hex(0x2E4D89),
    control_border: hex(0x40434A),
    control_bg: hex(0x191A1C),
    accent: hex(0x3871E1),
    grid: hex(0x2B2D31),
    header_bg: hex(0x212326),
    popup_bg: hex(0x26282C),
    popup_border: hex(0x33353B),
    stripe_selected: hex(0x40434A),
    badge_bg: hex(0x33353B),
    badge_fg: hex(0xB5B7BD),
    editor_bg: hex(0x191A1C),
    gutter_fg: hex(0x808080),
    gutter_fg_caret: hex(0xD0D0D0),
    caret_row: hex(0x202424),
    editor_selection: hex(0x08335E),
    error: hex(0xF57E84),
    ok: hex(0x6DB083),
    warn: hex(0xD59637),
    run: hex(0x5FB865),
    stop: hex(0xDB5C5C),
};

/// Editor colour scheme: "Rider Islands Dark" and "Rider Light" (`colorSchemes/*.xml` of
/// Rider 2026.2). Text attributes are (foreground, italic, bold).
#[derive(Clone, Copy)]
pub struct Scheme {
    pub text: Color32,
    pub identifier: Color32,
    pub keyword: Color32,
    pub string: Color32,
    pub escape: Color32,
    pub number: Color32,
    pub comment: Color32,
    pub doc_tag: Color32,
    pub class: Color32,
    pub struct_: Color32,
    pub method: Color32,
    pub field: Color32,
    pub event: Color32,
    pub punct: Color32,
    pub preproc: Color32,
    pub dimmed: Color32,
    pub xml_attr: Color32,
    pub entity: Color32,
    pub caret: Color32,
    pub indent_guide: Color32,
    pub indent_guide_selected: Color32,
    pub whitespace: Color32,
    pub right_margin: Color32,
    pub matched_brace: Color32,
    pub identifier_under_caret: Color32,
    pub search_result: Color32,
    pub search_result_current: Color32,
    pub folded_bg: Color32,
    pub folded_fg: Color32,
    pub folded_border: Color32,
    pub error: Color32,
    pub warning: Color32,
    pub info: Color32,
    pub hint_bg: Color32,
    pub hint_fg: Color32,
    pub hyperlink: Color32,
    pub breakpoint_line: Color32,
    pub execution_line: Color32,
    pub added_lines: Color32,
    pub modified_lines: Color32,
    pub deleted_lines: Color32,
    pub error_line: Color32,
    pub todo: Color32,
}

pub const SCHEME_LIGHT: Scheme = Scheme {
    text: hex(0x202020),
    identifier: hex(0x383838),
    keyword: hex(0x0F54D6),
    string: hex(0x8C6C41),
    escape: hex(0x941290),
    number: hex(0xAB2F6B),
    comment: hex(0x248700),
    doc_tag: hex(0x8BC775),
    class: hex(0x6B2FBA),
    struct_: hex(0x300073),
    method: hex(0x00855F),
    field: hex(0x0093A1),
    event: hex(0xAB2F6B),
    punct: hex(0x383838),
    preproc: hex(0x0F54D6),
    dimmed: hex(0x949494),
    xml_attr: hex(0x248700),
    entity: hex(0x635237),
    caret: hex(0x000000),
    indent_guide: hex(0xD8D8D8),
    indent_guide_selected: hex(0xA0A0A0),
    whitespace: hex(0xD0D0D0),
    right_margin: hex(0xD8D8D8),
    matched_brace: hex(0xFFE8FE),
    identifier_under_caret: hex(0xDAE4F7),
    search_result: hex(0xF1E7BC),
    search_result_current: hex(0xA9E6D0),
    folded_bg: hex(0xFBFBFB),
    folded_fg: hex(0x949494),
    folded_border: hex(0xD8D8D8),
    error: hex(0xD91400),
    warning: hex(0xCC9C00),
    info: hex(0x949494),
    hint_bg: hex(0xEBEBEB),
    hint_fg: hex(0x949494),
    hyperlink: hex(0x0F54D6),
    breakpoint_line: hex(0xFFEAE8),
    execution_line: hex(0xFFEAC4),
    added_lines: hex(0xBFDEB3),
    modified_lines: hex(0xC0DAF9),
    deleted_lines: hex(0xFF5C5C),
    error_line: hex(0xFFEAE8),
    todo: hex(0x0093A1),
};

pub const SCHEME_DARK: Scheme = Scheme {
    text: hex(0xD0D0D0),
    identifier: hex(0xBDBDBD),
    keyword: hex(0x6C95EB),
    string: hex(0xC9A26D),
    escape: hex(0xD688D4),
    number: hex(0xED94C0),
    comment: hex(0x85C46C),
    doc_tag: hex(0x487D34),
    class: hex(0xC191FF),
    struct_: hex(0xE1BFFF),
    method: hex(0x39CC9B),
    field: hex(0x66C3CC),
    event: hex(0xED94C0),
    punct: hex(0xBDBDBD),
    preproc: hex(0x6C95EB),
    dimmed: hex(0x787878),
    xml_attr: hex(0x85C46C),
    entity: hex(0xFFD49E),
    caret: hex(0xF0F0F0),
    indent_guide: hex(0x404040),
    indent_guide_selected: hex(0x666666),
    whitespace: hex(0x686868),
    right_margin: hex(0x404040),
    matched_brace: hex(0x422240),
    identifier_under_caret: hex(0x232E46),
    search_result: hex(0x484012),
    search_result_current: hex(0x144238),
    folded_bg: hex(0x303030),
    folded_fg: hex(0x787878),
    folded_border: hex(0x404040),
    error: hex(0xFF5647),
    warning: hex(0xF5D86A),
    info: hex(0x787878),
    hint_bg: hex(0x343434),
    hint_fg: hex(0x787878),
    hyperlink: hex(0x6C95EB),
    breakpoint_line: hex(0x4A2421),
    execution_line: hex(0x6B4C1B),
    added_lines: hex(0x2C4722),
    modified_lines: hex(0x293A5F),
    deleted_lines: hex(0xBF3428),
    error_line: hex(0x4A2421),
    todo: hex(0x66C3CC),
};

pub fn scheme(dark: bool) -> Scheme {
    if dark { SCHEME_DARK } else { SCHEME_LIGHT }
}

pub fn palette(dark: bool) -> Palette {
    if dark { DARK } else { LIGHT }
}

/// Palette for the `Ui`'s mode.
pub fn pal(ui: &Ui) -> Palette {
    palette(ui.visuals().dark_mode)
}

// ------------------------------------------------------------------ fonts

pub const FONT_BOLD: &str = "bold";
pub const FONT_MONO_BOLD: &str = "mono-bold";
pub const FONT_MONO_ITALIC: &str = "mono-italic";

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(FONT_BOLD.into()))
}

pub fn mono_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(FONT_MONO_BOLD.into()))
}

/// System fonts for the scripts Inter and JetBrains Mono lack (Chinese, Japanese, Korean,
/// Thai…), in order. Each group uses the first file that exists; they are read from disk, not
/// bundled (a CJK font weighs 20 MB). `%WINDIR%` is the Windows folder.
const SYSTEM_FONTS: &[&[&str]] = &[
    // CJK and Hangul; on macOS also Thai, Hebrew, Arabic, Devanagari…
    &[
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "%WINDIR%/Fonts/msyh.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    ],
    // Korean (Microsoft YaHei has no Hangul).
    &["%WINDIR%/Fonts/malgun.ttf"],
    // Thai.
    &["%WINDIR%/Fonts/LeelawUI.ttf", "/usr/share/fonts/truetype/noto/NotoSansThai-Regular.ttf"],
];

/// The system fallback fonts found, read once per run.
fn system_fonts() -> &'static [(String, Arc<FontData>)] {
    static FONTS: std::sync::OnceLock<Vec<(String, Arc<FontData>)>> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        let windir = std::env::var("WINDIR").or_else(|_| std::env::var("SystemRoot")).unwrap_or_else(|_| r"C:\Windows".into());
        SYSTEM_FONTS
            .iter()
            .filter_map(|group| {
                group.iter().find_map(|path| {
                    let path = path.replace("%WINDIR%", &windir);
                    let bytes = std::fs::read(&path).ok()?;
                    Some((format!("system:{path}"), Arc::new(FontData::from_owned(bytes))))
                })
            })
            .collect()
    })
}

/// Inter (UI) and JetBrains Mono (editor and grid). egui's default fonts remain as
/// fallback for symbols and emoji, and system fonts for CJK, Hangul, Thai…
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add(&mut fonts, "Inter", include_bytes!("../../../assets/fonts/Inter-Regular.otf"));
    add(&mut fonts, "Inter-SemiBold", include_bytes!("../../../assets/fonts/Inter-SemiBold.otf"));
    add(&mut fonts, "JBMono", include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf"));
    add(&mut fonts, "JBMono-Bold", include_bytes!("../../../assets/fonts/JetBrainsMono-Bold.ttf"));
    add(&mut fonts, "JBMono-Italic", include_bytes!("../../../assets/fonts/JetBrainsMono-Italic.ttf"));

    let mut fallback_prop = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut fallback_mono = fonts.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    for (name, data) in system_fonts() {
        fonts.font_data.insert(name.clone(), data.clone());
        fallback_prop.push(name.clone());
        fallback_mono.push(name.clone());
    }
    let with = |first: &str, rest: &[String]| -> Vec<String> {
        let mut v = vec![first.to_owned()];
        v.extend(rest.iter().cloned());
        v
    };
    fonts.families.insert(FontFamily::Proportional, with("Inter", &fallback_prop));
    fonts.families.insert(FontFamily::Monospace, with("JBMono", &fallback_mono));
    fonts.families.insert(FontFamily::Name(FONT_BOLD.into()), with("Inter-SemiBold", &fallback_prop));
    fonts.families.insert(FontFamily::Name(FONT_MONO_BOLD.into()), with("JBMono-Bold", &fallback_mono));
    fonts.families.insert(FontFamily::Name(FONT_MONO_ITALIC.into()), with("JBMono-Italic", &fallback_mono));
    ctx.set_fonts(fonts);
}

// ------------------------------------------------------------------ visuals

pub fn apply(ctx: &egui::Context, dark: bool) {
    let p = palette(dark);
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.main_bg;
    v.window_fill = p.popup_bg;
    v.extreme_bg_color = p.control_bg;
    v.text_edit_bg_color = Some(p.control_bg);
    v.faint_bg_color = p.header_bg;
    v.code_bg_color = p.editor_bg;
    v.window_stroke = Stroke::new(1.0, p.popup_border);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_shadow = Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(if dark { 110 } else { 38 }) };
    v.popup_shadow = Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if dark { 90 } else { 28 }) };
    v.override_text_color = None;
    v.hyperlink_color = p.accent;
    v.error_fg_color = p.error;
    v.warn_fg_color = p.warn;
    v.selection.bg_fill = p.editor_selection;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.striped = false;
    v.interact_cursor = None;
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = false;
    v.handle_shape = egui::style::HandleShape::Circle;

    let r = CornerRadius::same(6);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.island_bg;
    w.noninteractive.weak_bg_fill = p.island_bg;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.island_border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = p.control_bg;
    w.inactive.weak_bg_fill = p.control_bg;
    w.inactive.bg_stroke = Stroke::new(1.0, p.control_border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = p.control_bg;
    w.hovered.weak_bg_fill = blend(p.control_bg, p.hover);
    w.hovered.bg_stroke = Stroke::new(1.0, if dark { hex(0x5F6269) } else { hex(0xB5B7BD) });
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.control_bg;
    w.active.weak_bg_fill = blend(p.control_bg, p.pressed);
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.control_bg;
    w.open.weak_bg_fill = blend(p.control_bg, p.hover);
    w.open.bg_stroke = Stroke::new(1.0, p.accent);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = r;

    let theme = if dark { egui::Theme::Dark } else { egui::Theme::Light };
    ctx.set_theme(theme);
    ctx.set_visuals_of(theme, v);
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(13.0)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Heading, bold(15.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        s.spacing.item_spacing = vec2(6.0, 4.0);
        s.spacing.button_padding = vec2(8.0, 3.0);
        s.spacing.interact_size.y = 24.0;
        s.spacing.menu_margin = Margin::same(6);
        s.spacing.window_margin = Margin::same(16);
        s.spacing.combo_height = 300.0;
        s.spacing.icon_width = 14.0;
        s.spacing.icon_width_inner = 8.0;
        s.spacing.scroll = scroll_style();
        s.interaction.selectable_labels = false;
    });
}

/// IntelliJ-style scroll bars: thin, gray and rounded over a transparent track. They float
/// over the content (no layout change) but are always visible when there is something to
/// scroll; they widen on hover.
pub fn scroll_style() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        floating: true,
        bar_width: 9.0,
        floating_width: 6.0,
        floating_allocated_width: 0.0,
        bar_inner_margin: 2.0,
        bar_outer_margin: 2.0,
        handle_min_length: 24.0,
        // Text color (not `bg_fill`, which in Islands Light is white like the island).
        foreground_color: true,
        dormant_background_opacity: 0.0,
        active_background_opacity: 0.0,
        interact_background_opacity: 0.0,
        dormant_handle_opacity: 0.26,
        active_handle_opacity: 0.36,
        interact_handle_opacity: 0.5,
        ..egui::style::ScrollStyle::floating()
    }
}

/// Blends a translucent color over an opaque one.
pub fn blend(base: Color32, over: Color32) -> Color32 {
    let a = over.a() as f32 / 255.0;
    if a <= 0.0 {
        return base;
    }
    // `over` is premultiplied.
    let mix = |b: u8, o: u8| (b as f32 * (1.0 - a) + o as f32).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(mix(base.r(), over.r()), mix(base.g(), over.g()), mix(base.b(), over.b()))
}

// ------------------------------------------------------------------ widget helpers

/// Toolbar icon button (ActionButton): 26x26, rounded hover.
pub fn icon_button(ui: &mut Ui, ico: Ico, enabled: bool, tooltip: &str) -> Response {
    icon_button_sized(ui, ico, enabled, tooltip, 26.0)
}

pub fn icon_button_sized(ui: &mut Ui, ico: Ico, enabled: bool, tooltip: &str, size: f32) -> Response {
    let p = pal(ui);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), sense);
    if !tooltip.is_empty() { resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, false, tooltip)); }
    if enabled && (resp.hovered() || resp.is_pointer_button_down_on()) {
        let fill = if resp.is_pointer_button_down_on() { p.pressed } else { p.hover };
        ui.painter().rect_filled(rect, 6.0, fill);
    }
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(16.0)), ico, &p, enabled);
    if tooltip.is_empty() { resp } else { resp.on_hover_text(tooltip) }
}

/// Flat text button (with optional chevron) in the MainToolbar.Dropdown style.
pub fn flat_button(ui: &mut Ui, leading: Option<Ico>, text: &str, font: FontId, chevron: bool, enabled: bool) -> Response {
    let p = pal(ui);
    let color = if enabled { p.text } else { p.disabled };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let lead_w = if leading.is_some() { 22.0 } else { 0.0 };
    let chev_w = if chevron { 16.0 } else { 0.0 };
    let w = 8.0 + lead_w + galley.size().x + chev_w + 8.0;
    let h = 26.0;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), sense);
    if enabled && (resp.hovered() || resp.is_pointer_button_down_on()) {
        let fill = if resp.is_pointer_button_down_on() { p.pressed } else { p.hover };
        ui.painter().rect_filled(rect, 6.0, fill);
    }
    let mut x = rect.left() + 8.0;
    if let Some(ico) = leading {
        icons::paint(ui.painter(), Rect::from_min_size(egui::pos2(x, rect.center().y - 8.0), Vec2::splat(16.0)), ico, &p, enabled);
        x += lead_w;
    }
    ui.painter().galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley.clone(), color);
    x += galley.size().x + 2.0;
    if chevron {
        icons::paint(ui.painter(), Rect::from_min_size(egui::pos2(x, rect.center().y - 7.0), Vec2::splat(14.0)), ico::CHEVRON_DOWN, &p, enabled);
    }
    resp
}

/// Compact icon-and-label action, with a subtle selected state.
pub fn tool_button(ui: &mut Ui, icon: Ico, text: &str, selected: bool) -> Response {
    text_tool_button(ui, icon, text, selected, false)
}

pub fn add_button(ui: &mut Ui, icon: Ico, text: &str) -> Response {
    text_tool_button(ui, icon, text, false, true)
}

fn text_tool_button(ui: &mut Ui, icon: Ico, text: &str, selected: bool, accented: bool) -> Response {
    let p = pal(ui);
    let text_color = if !ui.is_enabled() { p.disabled } else if accented { p.accent } else { p.text };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(12.5), text_color);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 32.0, 24.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), selected, text));
    if selected {
        ui.painter().rect(rect, 4.0, p.tab_selected_bg, Stroke::new(1.0, p.tab_selected_border), egui::StrokeKind::Inside);
    } else if accented {
        ui.painter().rect(rect, 4.0, blend(p.island_bg, p.accent.gamma_multiply(if response.hovered() { 0.22 } else { 0.12 })), Stroke::new(1.0, p.accent.gamma_multiply(0.5)), egui::StrokeKind::Inside);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, p.hover);
    }
    icons::paint(ui.painter(), Rect::from_min_size(rect.left_center() + vec2(6.0, -7.0), Vec2::splat(14.0)), icon, &p, ui.is_enabled());
    ui.painter().galley(rect.left_center() + vec2(25.0, -galley.size().y / 2.0), galley, p.text);
    response
}

pub fn icon_tab(ui: &mut Ui, icon: Ico, label: &str, selected: bool) -> Response {
    let p = pal(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), selected, label));
    if selected || response.hovered() {
        ui.painter().rect_filled(rect, 4.0, if selected { p.tab_selected_bg } else { p.hover });
    }
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(15.0)), icon, &p, ui.is_enabled());
    response.on_hover_text(label)
}

/// Thin vertical toolbar separator.
pub fn toolbar_separator(ui: &mut Ui) {
    let p = pal(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 18.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, if p.dark { hex(0x393B40) } else { hex(0xDDDFE4) }));
}

/// Blue primary button (Button.default).
/// A menu row (New UI popup menus): 16 pt icon column (kept empty without an icon so the texts
/// line up), the text, and the shortcut right-aligned in grey.
pub fn menu_entry(ui: &mut Ui, icon: Option<crate::Ico>, text: &str, shortcut: Option<&str>, selected: bool, enabled: bool) -> Response {
    let p = pal(ui);
    let font = FontId::proportional(13.0);
    let fg = if enabled { p.text } else { p.disabled };
    let g = ui.painter().layout_no_wrap(text.to_string(), font.clone(), fg);
    let sc = shortcut.map(|s| ui.painter().layout_no_wrap(s.to_string(), FontId::proportional(12.5), p.muted));
    let want = 34.0 + g.size().x + sc.as_ref().map_or(0.0, |s| 28.0 + s.size().x) + 12.0;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (r, resp) = ui.allocate_exact_size(egui::vec2(want.max(ui.available_width()), 24.0), sense);
    if ui.is_rect_visible(r) {
        if selected {
            ui.painter().rect_filled(r, 4.0, p.selection);
        } else if enabled && resp.hovered() {
            ui.painter().rect_filled(r, 4.0, p.row_hover);
        }
        if let Some(ico) = icon {
            crate::icons::paint(ui.painter(), Rect::from_center_size(egui::pos2(r.left() + 16.0, r.center().y), egui::vec2(16.0, 16.0)), ico, &p, enabled);
        }
        ui.painter().galley(egui::pos2(r.left() + 34.0, (r.center().y - g.size().y / 2.0).round()), g, fg);
        if let Some(sc) = sc {
            ui.painter().galley(egui::pos2(r.right() - 10.0 - sc.size().x, (r.center().y - sc.size().y / 2.0).round()), sc, p.muted);
        }
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, text));
    resp
}

/// [`menu_entry`] without a shortcut.
pub fn menu_item(ui: &mut Ui, icon: Option<crate::Ico>, text: &str, selected: bool) -> Response {
    menu_entry(ui, icon, text, None, selected, true)
}

/// A slider readable on both themes: grey rail, accent fill up to the value and a white knob
/// with a border (egui draws the rail with the control background, white on Islands Light).
pub fn slider(ui: &mut Ui, slider: egui::Slider<'_>) -> Response {
    let p = pal(ui);
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = 200.0;
        let v = ui.visuals_mut();
        v.selection.bg_fill = p.accent;
        // The rail and the idle knob share `inactive.bg_fill`; the knob gets a darker border and
        // turns white (light) or bright (dark) when hovered or dragged.
        v.widgets.inactive.bg_fill = if p.dark { p.control_border } else { hex(0xD3D5DB) };
        v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, p.muted);
        for w in [&mut v.widgets.hovered, &mut v.widgets.active] {
            w.bg_fill = if p.dark { p.text } else { Color32::WHITE };
            w.fg_stroke = egui::Stroke::new(1.0, p.accent);
        }
        ui.add(slider.trailing_fill(true))
    })
    .inner
}

pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let p = pal(ui);
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(Color32::WHITE))
            .fill(p.accent)
            .stroke(Stroke::new(1.0, p.accent))
            .corner_radius(6)
            .min_size(vec2(72.0, 28.0)),
    )
}

/// Secondary button (control-border outline).
pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(egui::Button::new(text).corner_radius(6).min_size(vec2(72.0, 28.0)))
}

/// Rounded gray badge (List.Tag).
pub fn paint_badge(ui: &Ui, left_center: egui::Pos2, text: &str) -> Rect {
    let p = pal(ui);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(11.0), p.badge_fg);
    let rect = Rect::from_min_size(
        egui::pos2(left_center.x, left_center.y - 9.0),
        vec2(galley.size().x + 12.0, 18.0),
    );
    ui.painter().rect_filled(rect, 5.0, p.badge_bg);
    ui.painter().galley(egui::pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0), galley, p.badge_fg);
    rect
}

/// "Island" frame: rounded card with the tool window background.
pub fn island_frame(p: &Palette) -> egui::Frame {
    egui::Frame::NONE.fill(p.island_bg).corner_radius(CornerRadius::same(10))
}

/// Island that fills exactly all the available space of `ui` (e.g. a resizable panel):
/// the content is clipped inside and never grows the island or the panel, so the rounded
/// corners and the gap between islands are kept at any size.
pub fn fixed_island<R>(ui: &mut Ui, p: &Palette, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, CornerRadius::same(10), p.island_bg);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let r = add_contents(&mut child);
    ui.advance_cursor_after_rect(rect);
    r
}

/// Fixed-height row that cannot widen its parent: whatever does not fit is clipped.
pub fn clipped_row<R>(ui: &mut Ui, height: f32, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width().max(0.0), height));
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let r = add_contents(&mut child);
    ui.advance_cursor_after_rect(rect);
    r
}

/// Side of a row split with `split_row`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Row with a left group and a right-aligned group that never overlap: the right side is
/// drawn first (it has priority) and the left one takes what is left, clipped if it does
/// not fit. `add` is called twice, once per side.
pub fn split_row(ui: &mut Ui, mut add: impl FnMut(&mut Ui, Side)) {
    let rect = ui.available_rect_before_wrap();
    let clip = ui.clip_rect();
    let mut right =
        ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::right_to_left(egui::Align::Center)));
    right.set_clip_rect(rect.intersect(clip));
    add(&mut right, Side::Right);
    let right_w = right.min_rect().width();
    let left_right = (rect.right() - right_w - 8.0).max(rect.left());
    let left_rect = Rect::from_min_max(rect.min, egui::pos2(left_right, rect.bottom()));
    let mut left =
        ui.new_child(egui::UiBuilder::new().max_rect(left_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    left.set_clip_rect(left_rect.intersect(clip));
    add(&mut left, Side::Left);
    ui.advance_cursor_after_rect(rect);
}

/// Half the gap between islands (Island.borderWidth = 6).
pub const ISLAND_GAP: i8 = 3;

/// Rounded tab (EditorTabs / tool window tabs). Returns the tab's response and whether its
/// "×" was clicked.
pub fn tab(ui: &mut Ui, selected: bool, ico: Option<Ico>, text: &str, closable: bool) -> (Response, bool) {
    tab_ex(ui, selected, ico, text, closable, TabStyle::default())
}

/// How an editor tab is drawn beyond its text: preview (italics), pinned (a pin instead of the
/// close button until hovered) and a colour mark (file colours).
#[derive(Clone, Copy, Debug, Default)]
pub struct TabStyle {
    pub italic: bool,
    pub pinned: bool,
    pub color: Option<Color32>,
    /// Drag to reorder.
    pub draggable: bool,
}

pub fn tab_ex(ui: &mut Ui, selected: bool, ico: Option<Ico>, text: &str, closable: bool, style: TabStyle) -> (Response, bool) {
    let p = pal(ui);
    let font = FontId::proportional(13.0);
    let mut job = egui::text::LayoutJob::default();
    job.append(text, 0.0, egui::TextFormat { font_id: font, color: p.text, italics: style.italic, ..Default::default() });
    let galley = ui.painter().layout_job(job);
    let icon_w = if ico.is_some() { 22.0 } else { 0.0 };
    let close_w = if closable || style.pinned { 22.0 } else { 0.0 };
    let w = 10.0 + icon_w + galley.size().x + close_w + 10.0;
    let sense = if style.draggable { Sense::click_and_drag() } else { Sense::click() };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), sense);
    // The text is painted, so screen readers (and UI tests) need it spelled out.
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, text));
    if let Some(c) = style.color {
        // File colour: a light tint behind the tab (Rider's file colours).
        ui.painter().rect_filled(rect, 6.0, c.gamma_multiply(if p.dark { 0.16 } else { 0.12 }));
    }
    if selected {
        ui.painter().rect(rect, 6.0, p.tab_selected_bg, Stroke::new(1.0, p.tab_selected_border), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, p.hover);
    }
    if let Some(c) = style.color {
        let bar = Rect::from_min_max(egui::pos2(rect.left() + 8.0, rect.bottom() - 3.0), egui::pos2(rect.right() - 8.0, rect.bottom() - 1.0));
        ui.painter().rect_filled(bar, 1.0, c);
    }
    let mut x = rect.left() + 10.0;
    if let Some(ico) = ico {
        icons::paint(ui.painter(), Rect::from_min_size(egui::pos2(x, rect.center().y - 8.0), Vec2::splat(16.0)), ico, &p, true);
        x += icon_w;
    }
    ui.painter().galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley.clone(), p.text);
    x += galley.size().x + 4.0;
    let mut close = false;
    if closable || style.pinned {
        let cr = Rect::from_min_size(egui::pos2(x, rect.center().y - 8.0), Vec2::splat(16.0));
        let cresp = ui.interact(cr, resp.id.with("close"), Sense::click());
        if cresp.hovered() {
            ui.painter().rect_filled(cr.expand(1.0), 4.0, p.hover);
        }
        let hovered = resp.hovered() || cresp.hovered();
        if style.pinned && !(closable && hovered) {
            // Pinned: a small pin where the close button goes (the close shows on hover).
            icons::paint(ui.painter(), cr.shrink(1.0), ico::PIN, &p, true);
        } else if selected || hovered {
            icons::paint(ui.painter(), cr.shrink(2.0), ico::CLOSE_SMALL, &p, true);
        }
        close = closable && cresp.clicked();
    }
    (resp, close)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #4: CJK, Hangul and Thai in the grid and the UI used to show as boxes.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn system_fonts_cover_cjk_hangul_and_thai() {
        assert!(!system_fonts().is_empty(), "no system fallback font found");
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
        for family in [FontFamily::Proportional, FontFamily::Monospace, FontFamily::Name(FONT_BOLD.into())] {
            let font = FontId::new(13.0, family);
            for text in ["中文汉字", "日本語のテキスト", "한국어", "ภาษาไทย"] {
                assert!(ctx.fonts_mut(|f| f.has_glyphs(&font, text)), "{text} missing in {font:?}");
            }
        }
    }
}
