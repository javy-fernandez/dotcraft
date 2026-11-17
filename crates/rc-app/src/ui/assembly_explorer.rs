//! The Assembly Explorer tool window: assemblies › namespaces › types; a type opens as
//! decompiled C# (or IL) in a read-only editor tab.

use crate::app::DotCraftApp;
use crate::assembly_explorer::{AssemblyType, TypeKind};
use crate::toolwindow::ToolWindow;
use crate::ui::{kit, run_windows, toolwindows};
use egui::{Align2, FontId, Rect, Response, Sense, Ui, pos2, vec2};
use rc_kit::i18n::{tr, trf};
use rc_kit::{Ico, ico, icons};
use std::path::PathBuf;

enum Pick {
    Toggle(String),
    Select(String),
    Open(PathBuf, AssemblyType, bool),
    Remove(PathBuf),
    CopyPath(PathBuf),
}

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    let selected_assembly = app.assembly_explorer.selected.as_deref().and_then(|k| k.strip_prefix("a:")).map(PathBuf::from);
    let actions = [
        (ico::OPEN, tr("Open Assembly…"), true),
        (ico::LIBRARY, tr("Open Framework Assembly…"), true),
        (ico::REMOVE, tr("Remove from Assembly Explorer"), selected_assembly.is_some()),
        (ico::COLLAPSE_ALL, tr("Collapse All"), !app.assembly_explorer.expanded.is_empty()),
    ];
    match run_windows::header(ui, tr("Assembly Explorer"), &actions) {
        Some(usize::MAX) => {
            app.toggle_tool(ToolWindow::AssemblyExplorer);
            return;
        }
        Some(0) => app.open_assembly_dialog(None),
        Some(1) => {
            let dir = app.framework_assembly_dir(None);
            app.open_assembly_dialog(dir);
        }
        Some(2) => {
            if let Some(path) = &selected_assembly {
                app.assembly_explorer.remove(path);
            }
        }
        Some(3) => app.assembly_explorer.expanded.clear(),
        _ => {}
    }
    let x = &mut app.assembly_explorer;
    if x.tool_missing {
        let hint = tr("DotCraft decompiles assemblies with ilspycmd, the ILSpy command-line tool, installed as a .NET global tool.");
        let label = if x.installing { tr("Installing…") } else { tr("Install ilspycmd") };
        if toolwindows::empty_state_with(ui, ico::DOWNLOAD, tr("ilspycmd is not installed"), hint, Some(label)) && !x.installing {
            let outs = app.runner.run_tool(tr("Install ilspycmd").into(), crate::assembly_explorer::install_args(), None, crate::runner::ToolFollowup::RecheckIlspy);
            app.assembly_explorer.installing = app.runner.build.as_ref().is_some_and(|b| b.tool == Some(crate::runner::ToolFollowup::RecheckIlspy) && b.handle.is_some());
            run_windows::handle_outputs(app, outs);
        }
        return;
    }
    if x.assemblies.is_empty() && x.resolving.is_none() && x.error.is_none() {
        toolwindows::empty_state(ui, ico::LIBRARY, tr("No assemblies"), tr("Open a .dll or .exe, or choose Explore Assembly on a NuGet package in the Solution Explorer."));
        return;
    }
    if !x.assemblies.is_empty() {
        kit::toolbar(ui, |ui| {
            kit::search(ui, &mut x.filter, tr("Filter types"), (ui.available_width() - 4.0).clamp(120.0, 320.0));
        });
    }
    let status = match (&x.resolving, &x.decompiling) {
        (Some(name), _) => Some(trf("Looking up {name}…", &[("name", name)])),
        (_, Some(name)) => Some(trf("Decompiling {name}…", &[("name", name)])),
        _ => None,
    };
    if status.is_some() || x.error.is_some() {
        kit::content(ui, |ui| {
            if let Some(status) = &status {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(12.0));
                    kit::note(ui, status);
                });
            }
            if let Some(error) = &x.error {
                kit::message(ui, error, true);
            }
        });
    }
    let filter = x.filter.trim().to_lowercase();
    let single_click = app.settings.single_click_open;
    let mut picks = Vec::new();
    egui::ScrollArea::both().id_salt("assembly_explorer").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for assembly in &x.assemblies {
            let key = format!("a:{}", assembly.path.display());
            let open = x.expanded.contains(&key) || !filter.is_empty();
            let secondary = match (&assembly.types, &assembly.error) {
                (_, Some(_)) => tr("Could not be read").to_string(),
                (Some(types), _) => trf("{count} types", &[("count", &types.len())]),
                (None, None) => tr("Loading…").to_string(),
            };
            let resp = tree_row(ui, 0, assembly.types.as_ref().map(|t| (!t.is_empty(), open)), ico::LIBRARY, &assembly.name(), &secondary, x.selected.as_deref() == Some(&key));
            let resp = resp.on_hover_text(assembly.path.display().to_string());
            row_clicks(&resp, &key, 0, &mut picks);
            resp.context_menu(|ui| {
                if rc_kit::theme::menu_item(ui, Some(ico::REMOVE), tr("Remove from Assembly Explorer"), false).clicked() {
                    picks.push(Pick::Remove(assembly.path.clone()));
                    ui.close();
                }
                if rc_kit::theme::menu_item(ui, Some(ico::COPY), tr("Copy Path"), false).clicked() {
                    picks.push(Pick::CopyPath(assembly.path.clone()));
                    ui.close();
                }
            });
            if let Some(error) = &assembly.error
                && open
            {
                ui.horizontal(|ui| {
                    ui.add_space(42.0);
                    kit::message(ui, error, true);
                });
            }
            if !open {
                continue;
            }
            for (namespace, types) in assembly.namespaces() {
                let types: Vec<&AssemblyType> = types.into_iter().filter(|t| filter.is_empty() || t.full_name.to_lowercase().contains(&filter)).collect();
                if types.is_empty() {
                    continue;
                }
                let ns_key = format!("n:{}|{namespace}", assembly.path.display());
                let ns_open = x.expanded.contains(&ns_key) || !filter.is_empty();
                let label = if namespace.is_empty() { tr("<global namespace>") } else { namespace };
                let resp = tree_row(ui, 1, Some((true, ns_open)), ico::NAMESPACE, label, "", x.selected.as_deref() == Some(&ns_key));
                row_clicks(&resp, &ns_key, 1, &mut picks);
                if !ns_open {
                    continue;
                }
                for ty in types {
                    let ty_key = format!("t:{}|{}", assembly.path.display(), ty.full_name);
                    let resp = tree_row(ui, 2, None, type_icon(ty.kind), &ty.name, "", x.selected.as_deref() == Some(&ty_key));
                    let resp = resp.on_hover_text(&ty.full_name);
                    if resp.clicked() {
                        picks.push(Pick::Select(ty_key.clone()));
                    }
                    if resp.double_clicked() || (single_click && resp.clicked()) {
                        picks.push(Pick::Open(assembly.path.clone(), ty.clone(), false));
                    }
                    resp.context_menu(|ui| {
                        if rc_kit::theme::menu_item(ui, Some(ico::FILE_CS), tr("Decompile to C#"), false).clicked() {
                            picks.push(Pick::Open(assembly.path.clone(), ty.clone(), false));
                            ui.close();
                        }
                        if rc_kit::theme::menu_item(ui, Some(ico::FILE_TEXT), tr("Show IL Code"), false).clicked() {
                            picks.push(Pick::Open(assembly.path.clone(), ty.clone(), true));
                            ui.close();
                        }
                    });
                }
            }
        }
    });
    for pick in picks {
        match pick {
            Pick::Toggle(key) => {
                if !x.expanded.remove(&key) {
                    x.expanded.insert(key.clone());
                }
                x.selected = Some(key);
            }
            Pick::Select(key) => x.selected = Some(key),
            Pick::Open(path, ty, il) => {
                x.selected = Some(format!("t:{}|{}", path.display(), ty.full_name));
                x.decompile(&path, &ty, il);
            }
            Pick::Remove(path) => x.remove(&path),
            Pick::CopyPath(path) => ui.ctx().copy_text(path.display().to_string()),
        }
    }
}

/// A click on an assembly or namespace row selects it; on the chevron or a double click it
/// also expands or collapses it.
fn row_clicks(resp: &Response, key: &str, depth: usize, picks: &mut Vec<Pick>) {
    let on_chevron = resp.interact_pointer_pos().is_some_and(|p| p.x < resp.rect.left() + 12.0 + depth as f32 * 18.0 + 18.0);
    if resp.double_clicked() || (resp.clicked() && on_chevron) {
        picks.push(Pick::Toggle(key.to_string()));
    } else if resp.clicked() {
        picks.push(Pick::Select(key.to_string()));
    }
}

fn type_icon(kind: TypeKind) -> Ico {
    match kind {
        TypeKind::Class => ico::CLASS,
        TypeKind::Interface => ico::INTERFACE,
        TypeKind::Struct => ico::STRUCT,
        TypeKind::Delegate => ico::LAMBDA,
        TypeKind::Enum => ico::ENUM,
    }
}

/// A tree row: indent, chevron (`Some((has_children, open))`), icon, text and grey secondary.
fn tree_row(ui: &mut Ui, depth: usize, chevron: Option<(bool, bool)>, icon: Ico, text: &str, secondary: &str, selected: bool) -> Response {
    let p = rc_kit::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(240.0), 24.0), Sense::click());
    if selected {
        ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.selection);
    } else if resp.hovered() {
        ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
    }
    let x = r.left() + 12.0 + depth as f32 * 18.0;
    if let Some((true, open)) = chevron {
        let c = if open { ico::CHEVRON_DOWN } else { ico::CHEVRON_RIGHT };
        icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 8.0, r.center().y), vec2(16.0, 16.0)), c, &p, true);
    }
    icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 26.0, r.center().y), vec2(16.0, 16.0)), icon, &p, true);
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(13.0), p.text);
    let tx = x + 40.0;
    ui.painter().galley(pos2(tx, (r.center().y - g.size().y / 2.0).round()), g.clone(), p.text);
    if !secondary.is_empty() {
        ui.painter().text(pos2(tx + g.size().x + 8.0, r.center().y), Align2::LEFT_CENTER, secondary, FontId::proportional(12.0), p.muted);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    resp
}
