//! Design surface for the common static controls shared by MAUI and Avalonia XAML.

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;
use crate::ui::run_windows::header;
use crate::xaml_preview::{Element, is_xaml};
use egui::Ui;
use rc_kit::i18n::tr;

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    let active = app.docs.active_doc().filter(|doc| is_xaml(&doc.path))
        .map(|doc| (doc.path.clone(), doc.editor.buffer.version(), doc.editor.buffer.snapshot()));
    if let Some((path, version, snapshot)) = active {
        app.xaml_preview.observe(&path, version, snapshot);
    } else if app.xaml_preview.path.is_some() {
        app.xaml_preview.clear();
    }
    app.xaml_preview.poll();
    let actions: [(rc_kit::Ico, &str, bool); 0] = [];
    if header(ui, tr("XAML Preview"), &actions) == Some(usize::MAX) {
        app.toggle_tool(ToolWindow::XamlPreview);
    }
    let Some(path) = &app.xaml_preview.path else {
        ui.label(tr("Open a XAML or AXAML file to preview it."));
        return;
    };
    ui.label(path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default());
    if app.xaml_preview.loading { ui.small(tr("Updating preview…")); }
    if let Some(error) = &app.xaml_preview.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
        return;
    }
    let Some(model) = app.xaml_preview.model.clone() else { return };
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(240.0);
            render(ui, &model);
        });
    });
}

fn render(ui: &mut Ui, element: &Element) {
    match element.tag.as_str() {
        "ContentPage" | "Window" | "UserControl" | "Grid" | "ScrollView" | "ContentView" | "VerticalStackLayout" | "StackLayout" | "StackPanel" => {
            if element.orientation.eq_ignore_ascii_case("Horizontal") {
                ui.horizontal_wrapped(|ui| render_children(ui, element));
            } else {
                ui.vertical(|ui| render_children(ui, element));
            }
        }
        "HorizontalStackLayout" | "WrapPanel" => { ui.horizontal_wrapped(|ui| render_children(ui, element)); }
        "Border" | "Frame" | "GroupBox" => { ui.group(|ui| render_children(ui, element)); }
        "Label" | "TextBlock" => { ui.label(&element.text); }
        "Button" => { let _ = ui.button(if element.text.is_empty() { tr("Button") } else { &element.text }); }
        "Entry" | "TextBox" | "Editor" => {
            let mut text = if element.text.is_empty() { element.placeholder.clone() } else { element.text.clone() };
            ui.add(egui::TextEdit::singleline(&mut text).interactive(false));
        }
        "CheckBox" => {
            let mut checked = false;
            ui.add_enabled(false, egui::Checkbox::new(&mut checked, &element.text));
        }
        "Image" => { ui.group(|ui| { ui.label(tr("Image")); }); }
        "BoxView" | "Rectangle" => { ui.separator(); }
        _ => {
            ui.group(|ui| {
                ui.small(&element.tag);
                render_children(ui, element);
            });
        }
    }
}

fn render_children(ui: &mut Ui, element: &Element) {
    if !element.text.is_empty() && !matches!(element.tag.as_str(), "Label" | "TextBlock" | "Button") {
        ui.label(&element.text);
    }
    for child in &element.children { render(ui, child); }
}
