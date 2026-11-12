use crate::app::DotCraftApp;
use egui::{Context, RichText, ScrollArea};
use rc_kit::i18n::tr;

pub fn show(app: &mut DotCraftApp, ctx: &Context) {
    let explorer = &mut app.assembly_explorer;
    explorer.poll();
    if !explorer.visible {
        return;
    }
    let mut open = true;
    let mut select_type = None;
    let mut change_mode = None;
    let mut open_framework = None;
    egui::Window::new(tr("Assembly Explorer"))
        .open(&mut open)
        .default_size(egui::vec2(1050.0, 700.0))
        .show(ctx, |ui| {
            if let Some(path) = &explorer.path {
                ui.label(path.display().to_string());
            }
            if let Some(error) = &explorer.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            if explorer.busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tr("Reading assembly…"));
                    if ui.button(tr("Cancel")).clicked() {
                        explorer.cancel();
                    }
                });
            }
            ui.separator();
            if ui.button(tr("Browse .NET reference assemblies")).clicked() {
                explorer.list_framework_assemblies();
            }
            if !explorer.framework_assemblies.is_empty() {
                ui.collapsing(tr("Reference assemblies"), |ui| {
                    ui.text_edit_singleline(&mut explorer.framework_filter);
                    let filter = explorer.framework_filter.to_lowercase();
                    ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                        for path in &explorer.framework_assemblies {
                            let label = path.file_name().unwrap_or_default().to_string_lossy();
                            if !filter.is_empty() && !label.to_lowercase().contains(&filter) {
                                continue;
                            }
                            if ui
                                .selectable_label(
                                    explorer.path.as_ref() == Some(path),
                                    label.as_ref(),
                                )
                                .on_hover_text(path.display().to_string())
                                .clicked()
                            {
                                open_framework = Some(path.clone());
                            }
                        }
                    });
                });
            }
            ui.separator();
            ui.columns(2, |columns| {
                let left = &mut columns[0];
                left.label(tr("Namespaces and types"));
                left.text_edit_singleline(&mut explorer.filter);
                let filter = explorer.filter.to_lowercase();
                ScrollArea::vertical()
                    .id_salt("assembly_types")
                    .show(left, |ui| {
                        let mut previous_namespace = String::new();
                        for ty in &explorer.types {
                            if !filter.is_empty() && !ty.full_name.to_lowercase().contains(&filter)
                            {
                                continue;
                            }
                            if previous_namespace != ty.namespace {
                                previous_namespace = ty.namespace.clone();
                                ui.label(
                                    RichText::new(if ty.namespace.is_empty() {
                                        tr("Global namespace")
                                    } else {
                                        &ty.namespace
                                    })
                                    .strong(),
                                );
                            }
                            let selected = explorer.selected.as_deref() == Some(&ty.full_name);
                            if ui
                                .selectable_label(selected, format!("{}  {}", ty.name, ty.kind))
                                .clicked()
                            {
                                select_type = Some(ty.full_name.clone());
                            }
                        }
                    });
                let right = &mut columns[1];
                right.horizontal(|ui| {
                    if ui.selectable_label(!explorer.show_il, "C#").clicked() && explorer.show_il {
                        change_mode = Some(false);
                    }
                    if ui.selectable_label(explorer.show_il, "IL").clicked() && !explorer.show_il {
                        change_mode = Some(true);
                    }
                    if ui.button(tr("All members")).clicked() {
                        explorer.selected_member = None;
                    }
                });
                if !explorer.members.is_empty() {
                    ScrollArea::horizontal()
                        .id_salt("assembly_members")
                        .max_height(34.0)
                        .show(right, |ui| {
                            ui.horizontal(|ui| {
                                for (index, member) in explorer.members.iter().enumerate() {
                                    if ui
                                        .selectable_label(
                                            explorer.selected_member == Some(index),
                                            &member.name,
                                        )
                                        .clicked()
                                    {
                                        explorer.selected_member = Some(index);
                                    }
                                }
                            });
                        });
                }
                let source = member_source(
                    &explorer.source,
                    &explorer.members,
                    explorer.selected_member,
                    explorer.show_il,
                );
                ScrollArea::both()
                    .id_salt("assembly_source")
                    .show(right, |ui| {
                        ui.label(RichText::new(source).monospace());
                    });
            });
        });
    if let Some(name) = select_type {
        explorer.select(name, explorer.show_il);
    }
    if let Some(path) = open_framework {
        explorer.open(path);
    }
    if let (Some(mode), Some(name)) = (change_mode, explorer.selected.clone()) {
        explorer.select(name, mode);
    }
    if !open {
        explorer.visible = false;
        explorer.cancel();
    }
}

fn member_source<'a>(
    source: &'a str,
    members: &[crate::assembly_explorer::Member],
    selected: Option<usize>,
    il: bool,
) -> &'a str {
    let Some(index) = selected else { return source };
    let Some(member) = members.get(index) else {
        return source;
    };
    if il {
        return source;
    }
    let start = source
        .lines()
        .take(member.line)
        .map(|line| line.len() + 1)
        .sum::<usize>()
        .min(source.len());
    let next = members
        .get(index + 1)
        .map_or(source.lines().count(), |member| member.line);
    let end = source
        .lines()
        .take(next)
        .map(|line| line.len() + 1)
        .sum::<usize>()
        .min(source.len());
    source.get(start..end).unwrap_or(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_selection_shows_only_its_source() {
        let source = "class Widget {\nvoid First() {}\nvoid Second() {}\n}";
        let members = vec![
            crate::assembly_explorer::Member {
                name: "First".into(),
                line: 1,
            },
            crate::assembly_explorer::Member {
                name: "Second".into(),
                line: 2,
            },
        ];
        assert_eq!(
            member_source(source, &members, Some(0), false),
            "void First() {}\n"
        );
        assert_eq!(member_source(source, &members, None, false), source);
    }
}
