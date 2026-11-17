//! A scrollable project and package reference graph for the Dependencies tool window.

use std::collections::VecDeque;
use std::path::PathBuf;

use egui::{Align2, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};
use rc_dotnet::dependency_graph::{DependencyGraph, DependencyNode};
use rc_kit::i18n::tr;
use rc_kit::{ico, icons, theme};

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;

const HEADER_H: f32 = 36.0;
const CARD_W: f32 = 184.0;
const CARD_H: f32 = 52.0;
const COL_W: f32 = 270.0;
const ROW_H: f32 = 76.0;
const PAD: f32 = 20.0;

enum Pick {
    Project(PathBuf),
    Package(PathBuf, String),
}

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    app.dependency_diagram.update();
    let palette = rc_kit::pal(ui);
    let header = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), HEADER_H));
    ui.allocate_rect(header, Sense::hover());
    ui.painter().text(pos2(header.left() + 12.0, header.center().y), Align2::LEFT_CENTER, tr("Dependencies"), theme::bold(13.0), palette.text);
    let refresh = Rect::from_min_size(pos2(header.right() - 62.0, header.center().y - 13.0), vec2(26.0, 26.0));
    let hide = Rect::from_min_size(pos2(header.right() - 34.0, header.center().y - 13.0), vec2(26.0, 26.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(refresh));
    if theme::icon_button(&mut child, ico::REFRESH, !app.dependency_diagram.loading, tr("Refresh")).clicked() {
        app.dependency_diagram.refresh();
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(hide));
    if theme::icon_button(&mut child, ico::HIDE, true, tr("Hide")).clicked() {
        app.toggle_tool(ToolWindow::DependencyDiagram);
        return;
    }
    if app.dependency_diagram.loading {
        ui.add_space(12.0);
        ui.label(tr("Loading…"));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        return;
    }
    let Some(graph) = app.dependency_diagram.graph.as_ref() else {
        ui.add_space(12.0);
        ui.label(tr("No projects"));
        return;
    };
    if graph.nodes.is_empty() {
        ui.add_space(12.0);
        ui.label(tr("No projects"));
        return;
    }
    let mut pick = None;
    egui::ScrollArea::both().id_salt("dependency_diagram_graph").auto_shrink([false, false]).show(ui, |ui| {
        let (cards, size) = layout(graph);
        let (canvas, _) = ui.allocate_exact_size(size, Sense::hover());
        let cards: Vec<_> = cards.into_iter().map(|card| card.translate(canvas.min.to_vec2())).collect();
        draw_edges(ui, graph, &cards);
        for (id, node) in graph.nodes.iter().enumerate() {
            let rect = cards[id];
            let response = ui.interact(rect, ui.id().with(("dependency_node", id)), Sense::click());
            let fill = if response.hovered() { palette.row_hover } else { palette.control_bg };
            ui.painter().rect_filled(rect, 6.0, palette.control_border);
            ui.painter().rect_filled(rect.shrink(1.0), 5.0, fill);
            let (icon, title, detail, tooltip) = match node {
                DependencyNode::Project { path, name, .. } => (ico::PROJECT, name.clone(), tr("Project").to_string(), path.display().to_string()),
                DependencyNode::Package { name, version } => (ico::PACKAGE, name.clone(), version.clone(), format!("{name} {version}")),
            };
            icons::paint(ui.painter(), Rect::from_min_size(rect.min + vec2(10.0, 17.0), vec2(16.0, 16.0)), icon, &palette, true);
            let text = ui.painter().with_clip_rect(rect.shrink2(vec2(6.0, 3.0)));
            text.text(pos2(rect.left() + 34.0, rect.top() + 17.0), Align2::LEFT_CENTER, title, FontId::proportional(12.5), palette.text);
            text.text(pos2(rect.left() + 34.0, rect.top() + 35.0), Align2::LEFT_CENTER, detail, FontId::proportional(11.0), palette.muted);
            response.clone().on_hover_text(tooltip);
            if response.clicked() {
                pick = match node {
                    DependencyNode::Project { path, .. } => Some(Pick::Project(path.clone())),
                    DependencyNode::Package { name, .. } => graph.edges.iter().find(|(_, target)| *target == id).and_then(|(source, _)| match &graph.nodes[*source] {
                        DependencyNode::Project { path, .. } => Some(Pick::Package(path.clone(), name.clone())),
                        _ => None,
                    }),
                };
            }
        }
    });
    match pick {
        Some(Pick::Project(path)) => app.open_file(&path),
        Some(Pick::Package(project, package)) => {
            app.nuget.focus_project(&project);
            app.nuget.open_package(&package);
            app.dock.show(ToolWindow::Nuget);
            app.session_dirty = true;
        }
        None => {}
    }
}

/// Place projects in reference-depth columns and packages after their deepest referencing
/// project. Cycles remain finite: each project is visited once.
fn layout(graph: &DependencyGraph) -> (Vec<Rect>, egui::Vec2) {
    let len = graph.nodes.len();
    let mut ranks = vec![0usize; len];
    let mut incoming = vec![0usize; len];
    let mut project_edges = vec![Vec::new(); len];
    for &(source, target) in &graph.edges {
        if matches!(graph.nodes[source], DependencyNode::Project { .. }) && matches!(graph.nodes[target], DependencyNode::Project { .. }) {
            incoming[target] += 1;
            project_edges[source].push(target);
        }
    }
    let mut seen = vec![false; len];
    let roots: Vec<_> = (0..len).filter(|id| matches!(graph.nodes[*id], DependencyNode::Project { .. }) && incoming[*id] == 0).collect();
    let mut queue = VecDeque::new();
    for root in roots.into_iter().chain(0..len) {
        if !matches!(graph.nodes[root], DependencyNode::Project { .. }) || seen[root] {
            continue;
        }
        queue.push_back(root);
        seen[root] = true;
        while let Some(source) = queue.pop_front() {
            for &target in &project_edges[source] {
                if !seen[target] {
                    ranks[target] = ranks[source] + 1;
                    seen[target] = true;
                    queue.push_back(target);
                }
            }
        }
    }
    for &(source, target) in &graph.edges {
        if matches!(graph.nodes[target], DependencyNode::Package { .. }) {
            ranks[target] = ranks[target].max(ranks[source] + 1);
        }
    }
    let mut rows = vec![0usize; ranks.iter().copied().max().unwrap_or(0) + 1];
    let cards = ranks.iter().map(|rank| {
        let row = rows[*rank];
        rows[*rank] += 1;
        Rect::from_min_size(pos2(PAD + *rank as f32 * COL_W, PAD + row as f32 * ROW_H), vec2(CARD_W, CARD_H))
    }).collect();
    let width = PAD * 2.0 + rows.len() as f32 * COL_W;
    let height = PAD * 2.0 + rows.into_iter().max().unwrap_or(0) as f32 * ROW_H;
    (cards, vec2(width, height))
}

fn draw_edges(ui: &Ui, graph: &DependencyGraph, cards: &[Rect]) {
    let color = rc_kit::pal(ui).muted.gamma_multiply(0.75);
    let stroke = Stroke::new(1.4, color);
    for &(source, target) in &graph.edges {
        let start = cards[source].right_center();
        let end = cards[target].left_center();
        let bend = if end.x > start.x + 16.0 { (start.x + end.x) / 2.0 } else { start.x + 26.0 };
        ui.painter().add(egui::Shape::line(vec![start, pos2(bend, start.y), pos2(bend, end.y), end], stroke));
        ui.painter().add(egui::Shape::line(vec![end + vec2(7.0, -4.0), end, end + vec2(7.0, 4.0)], stroke));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_package_is_one_card_reached_from_both_projects() {
        let graph = DependencyGraph {
            nodes: vec![
                DependencyNode::Project { path: "App.csproj".into(), name: "App".into(), in_solution: true },
                DependencyNode::Project { path: "Lib.csproj".into(), name: "Lib".into(), in_solution: true },
                DependencyNode::Package { name: "Example".into(), version: "1.0".into() },
            ],
            edges: vec![(0, 1), (0, 2), (1, 2)],
        };
        let (cards, _) = layout(&graph);
        assert_eq!(cards.len(), 3);
        assert!(cards[0].right() < cards[1].left());
        assert!(cards[1].right() < cards[2].left());
    }
}
