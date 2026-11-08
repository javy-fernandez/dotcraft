//! Monitoring tool window: CPU and memory charts of the running program and the .NET runtime's
//! counters, like Rider's Monitoring.

use crate::app::DotCraftApp;
use crate::monitor::{Monitor, MonitorTab, Sample};
use crate::toolwindow::ToolWindow;
use crate::ui::toolwindows::{HEADER_H, empty_state};
use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};
use rc_kit::i18n::{tr, trf};
use rc_kit::theme;

/// A chart line: points (seconds, value), colour and legend.
type Series<'a> = (&'a [(f64, f64)], Color32, &'a str);

/// Seconds shown in the charts.
const WINDOW: f64 = 120.0;

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    let m = &app.monitor;
    // Header: title, the process and its state, hide.
    let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), HEADER_H));
    ui.allocate_rect(r, Sense::hover());
    let title = ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, ToolWindow::Monitoring.title(), theme::bold(13.0), p.text);
    // Tabs: charts, memory snapshot, CPU profile.
    let mut x = title.right() + 14.0;
    let mut tab = m.tab;
    for (label, t, icon) in [(tr("Charts"), MonitorTab::Charts, rc_kit::ico::TW_MONITORING), (tr("Memory"), MonitorTab::Memory, rc_kit::ico::PACKAGE), (tr("CPU"), MonitorTab::Cpu, rc_kit::ico::RUN)] {
        let w = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.5), p.text).size().x + 32.0;
        let br = Rect::from_min_size(pos2(x, r.center().y - 12.0), vec2(w, 24.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
        if theme::tool_button(&mut child, icon, label, m.tab == t).clicked() {
            tab = t;
        }
        x += w + 6.0;
    }
    if let Some(pid) = m.app_pid.or(m.pid) {
        let state = if m.running { trf("{name} · PID {pid}", &[("name", &m.name), ("pid", &pid)]) } else { trf("{name} · exited", &[("name", &m.name)]) };
        ui.painter().text(pos2(x + 8.0, r.center().y), Align2::LEFT_CENTER, state, FontId::proportional(12.0), p.muted);
    }
    // Actions on the right: Hide, Start/Stop profiling, Get Snapshot.
    let can = m.target_pid().is_some();
    let mut action = None;
    for (i, (icon, tip, enabled)) in [
        (rc_kit::ico::HIDE, tr("Hide"), true),
        (if m.profiling() { rc_kit::ico::STOP } else { rc_kit::ico::RUN }, if m.profiling() { tr("Stop Profiling") } else { tr("Start CPU Profiling") }, can || m.profiling()),
        (rc_kit::ico::DOWNLOAD, tr("Get Memory Snapshot"), can && !m.snapshot_running()),
    ]
    .into_iter()
    .enumerate()
    {
        let br = Rect::from_min_size(pos2(r.right() - 8.0 - 26.0 * (i + 1) as f32, r.center().y - 13.0), vec2(26.0, 26.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
        if theme::icon_button(&mut child, icon, enabled, tip).clicked() && enabled {
            action = Some(i);
        }
    }
    let m = &mut app.monitor;
    m.tab = tab;
    match action {
        Some(0) => {
            app.toggle_tool(ToolWindow::Monitoring);
            return;
        }
        Some(1) if m.profiling() => m.stop_profiling(),
        Some(1) => m.start_profiling(),
        Some(2) => m.take_snapshot(),
        _ => {}
    }
    match m.tab {
        MonitorTab::Charts if m.samples.is_empty() => empty_state(ui, ToolWindow::Monitoring.icon(), tr("Nothing to monitor"), tr("Run a project to see its CPU, memory and garbage collections here.")),
        MonitorTab::Charts => charts(ui, m),
        MonitorTab::Memory => memory_tab(ui, m),
        MonitorTab::Cpu => cpu_tab(ui, m),
    }
}

fn message(ui: &mut Ui, text: &str, error: bool) {
    let p = rc_kit::pal(ui);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(text).size(12.5).color(if error { p.error } else { p.muted }));
    });
}

/// Memory snapshot: the heap's types by size (shallow), filterable.
fn memory_tab(ui: &mut Ui, m: &mut Monitor) {
    let p = rc_kit::pal(ui);
    if m.snapshot_running() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
        message(ui, tr("Taking a memory snapshot (it runs a full garbage collection)…"), false);
        return;
    }
    let snap = match &m.snapshot {
        None => {
            empty_state(ui, rc_kit::ico::PACKAGE, tr("No memory snapshot"), tr("Get a snapshot of the running program to see which types use its memory."));
            return;
        }
        Some(Err(e)) => {
            message(ui, e, true);
            return;
        }
        Some(Ok(s)) => s,
    };
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(trf("{objects} objects · {size}", &[("objects", &snap.total_objects), ("size", &bytes(snap.total_bytes as f64))])).strong());
        ui.add_space(16.0);
        ui.add(egui::TextEdit::singleline(&mut m.filter).hint_text(tr("Filter")).desired_width(240.0));
    });
    let f = m.filter.to_lowercase();
    let total = snap.total_bytes.max(1) as f64;
    table_header(ui, &[tr("Type"), tr("Objects"), tr("Size"), "%"]);
    egui::ScrollArea::vertical().id_salt("gcdump").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for t in snap.types.iter().filter(|t| f.is_empty() || t.name.to_lowercase().contains(&f)).take(1000) {
            let pc = t.size_bytes as f64 * 100.0 / total;
            table_row(ui, &t.name, &[t.count.to_string(), bytes(t.size_bytes as f64), format!("{pc:.1}")], pc, p.accent);
        }
    });
}

/// CPU profile: hot spots (methods by total time) or the call tree.
fn cpu_tab(ui: &mut Ui, m: &mut Monitor) {
    let p = rc_kit::pal(ui);
    if m.profiling() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        let secs = m.profile_started.map_or(0, |t| t.elapsed().as_secs());
        message(ui, &trf("Profiling… {secs} s. Stop to see where the time goes.", &[("secs", &secs)]), false);
        return;
    }
    let prof = match &m.profile {
        None => {
            empty_state(ui, rc_kit::ico::RUN, tr("No CPU profile"), tr("Start profiling the running program, use it for a while and stop to see its hot methods."));
            return;
        }
        Some(Err(e)) => {
            message(ui, e, true);
            return;
        }
        Some(Ok(pr)) => pr.clone(),
    };
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(trf("{n} samples", &[("n", &prof.samples)])).strong());
        ui.add_space(12.0);
        if theme::tool_button(ui, rc_kit::ico::MENU, tr("Hot Spots"), !m.show_tree).clicked() {
            m.show_tree = false;
        }
        if theme::tool_button(ui, rc_kit::ico::TW_HIERARCHY, tr("Call Tree"), m.show_tree).clicked() {
            m.show_tree = true;
        }
    });
    let total = prof.samples.max(1) as f64;
    if !m.show_tree {
        table_header(ui, &[tr("Method"), tr("Self"), tr("Total"), "%"]);
        egui::ScrollArea::vertical().id_salt("hot_methods").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for h in &prof.hot_methods {
                let pc = h.total_samples as f64 * 100.0 / total;
                table_row(ui, &h.name, &[format!("{:.1}%", h.self_samples as f64 * 100.0 / total), format!("{pc:.1}%"), String::new()], pc, p.warn);
            }
        });
        return;
    }
    let _ = p;
    let mut toggle: Option<String> = None;
    egui::ScrollArea::both().id_salt("call_tree").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        fn walk(ui: &mut Ui, n: &rc_dotnet::cpu_profile::CallNode, path: String, depth: usize, total: f64, expanded: &std::collections::HashSet<String>, toggle: &mut Option<String>) {
            let p = rc_kit::pal(ui);
            let open = depth == 0 || expanded.contains(&path);
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(500.0), 22.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, 3.0, p.row_hover);
            }
            let x0 = rect.left() + 10.0 + depth as f32 * 16.0;
            if !n.children.is_empty() {
                let ico = if open { rc_kit::ico::CHEVRON_DOWN } else { rc_kit::ico::CHEVRON_RIGHT };
                rc_kit::icons::paint(ui.painter(), Rect::from_center_size(pos2(x0 + 7.0, rect.center().y), vec2(14.0, 14.0)), ico, &p, true);
            }
            let pc = n.total_samples as f64 * 100.0 / total;
            let label = format!("{pc:.1}%  {}", n.name);
            ui.painter().text(pos2(x0 + 18.0, rect.center().y), Align2::LEFT_CENTER, &label, FontId::proportional(12.5), p.text);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &n.name));
            if resp.clicked() && depth > 0 {
                *toggle = Some(path.clone());
            }
            if open {
                for c in &n.children {
                    walk(ui, c, format!("{path}/{}", c.name), depth + 1, total, expanded, toggle);
                }
            }
        }
        walk(ui, &prof.call_tree, prof.call_tree.name.clone(), 0, total, &m.expanded, &mut toggle);
    });
    if let Some(t) = toggle
        && !m.expanded.remove(&t)
    {
        m.expanded.insert(t);
    }
}

fn table_header(ui: &mut Ui, cols: &[&str]) {
    let p = rc_kit::pal(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    let xs = columns(rect);
    for (i, c) in cols.iter().enumerate() {
        let align = if i == 0 { Align2::LEFT_CENTER } else { Align2::RIGHT_CENTER };
        ui.painter().text(pos2(xs[i], rect.center().y), align, *c, theme::bold(12.0), p.muted);
    }
    ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.island_border));
}

/// Column positions: the name on the left, numbers right-aligned on the right.
fn columns(rect: Rect) -> [f32; 4] {
    let right = rect.right() - 12.0;
    [rect.left() + 12.0, right - 300.0, right - 170.0, right - 40.0]
}

fn table_row(ui: &mut Ui, name: &str, cells: &[String], percent: f64, color: Color32) {
    let p = rc_kit::pal(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, p.row_hover);
    }
    let xs = columns(rect);
    let name_rect = Rect::from_min_max(rect.min, pos2(xs[1] - 90.0, rect.bottom()));
    ui.painter().with_clip_rect(name_rect).text(pos2(xs[0], rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(12.5), p.text);
    for (i, c) in cells.iter().enumerate() {
        ui.painter().text(pos2(xs[i + 1], rect.center().y), Align2::RIGHT_CENTER, c, FontId::monospace(12.0), p.text);
    }
    // A bar under the share.
    let bar = Rect::from_min_size(pos2(xs[3] + 6.0, rect.center().y - 3.0), vec2(30.0 * (percent / 100.0).clamp(0.0, 1.0) as f32, 6.0));
    ui.painter().rect_filled(bar, 1.0, color);
}

fn charts(ui: &mut Ui, m: &Monitor) {
    let p = rc_kit::pal(ui);
    let area = ui.available_rect_before_wrap().shrink2(vec2(12.0, 6.0));
    ui.allocate_rect(area, Sense::hover());
    let last = m.last().cloned().unwrap_or_default();
    let rt = m.runtime().cloned();
    // Two charts side by side and the runtime's counters on the right (or below when narrow).
    let stats_w = if area.width() > 760.0 { 230.0 } else { 0.0 };
    let charts_w = area.width() - stats_w - if stats_w > 0.0 { 12.0 } else { 0.0 };
    let chart_h = if stats_w > 0.0 { area.height() } else { (area.height() - 90.0).max(80.0) };
    let w = (charts_w - 12.0) / 2.0;
    let cpu_rect = Rect::from_min_size(area.min, vec2(w, chart_h));
    let mem_rect = Rect::from_min_size(pos2(cpu_rect.right() + 12.0, area.top()), vec2(w, chart_h));
    let blue = Color32::from_rgb(0x35, 0x74, 0xF0);
    let green = Color32::from_rgb(0x5F, 0xB8, 0x65);
    let orange = Color32::from_rgb(0xE0, 0x88, 0x55);
    let cpu: Vec<(f64, f64)> = m.samples.iter().map(|s| (s.t, s.cpu)).collect();
    chart(ui, cpu_rect, tr("CPU"), &format!("{:.0} %", last.cpu), &[(&cpu, blue, tr("Process"))], Some(100.0), "%");
    let mem: Vec<(f64, f64)> = m.samples.iter().map(|s| (s.t, s.memory_mb)).collect();
    let heap: Vec<(f64, f64)> = gc_heap_series(m.samples.iter());
    let mut series: Vec<Series> = vec![(&mem, green, tr("Working set"))];
    if !heap.is_empty() {
        series.push((&heap, orange, tr("GC heap")));
    }
    chart(ui, mem_rect, tr("Memory"), &format!("{:.0} MB", last.memory_mb), &series, None, "MB");

    let stats_rect = if stats_w > 0.0 { Rect::from_min_size(pos2(area.right() - stats_w, area.top()), vec2(stats_w, area.height())) } else { Rect::from_min_size(pos2(area.left(), cpu_rect.bottom() + 10.0), vec2(area.width(), 80.0)) };
    let fmt = |v: Option<f64>, f: &dyn Fn(f64) -> String| v.map_or_else(|| "—".to_string(), f);
    let rows: Vec<(&str, String)> = vec![
        (tr("GC heap"), fmt(rt.as_ref().and_then(|r| r.gc_heap_mb), &|v| format!("{v:.1} MB"))),
        (tr("Gen 0 / 1 / 2 collections"), match &rt {
            Some(r) if r.gen0.is_some() => format!("{} / {} / {}", fmt(r.gen0, &|v| format!("{v:.0}")), fmt(r.gen1, &|v| format!("{v:.0}")), fmt(r.gen2, &|v| format!("{v:.0}"))),
            _ => "—".into(),
        }),
        (tr("Time in GC"), fmt(rt.as_ref().and_then(|r| r.time_in_gc), &|v| format!("{v:.0} %"))),
        (tr("Allocation rate"), fmt(rt.as_ref().and_then(|r| r.alloc_rate), &|v| format!("{} /s", bytes(v)))),
        (tr("Thread pool threads"), fmt(rt.as_ref().and_then(|r| r.threadpool_threads), &|v| format!("{v:.0}"))),
        (tr("Exceptions"), fmt(rt.as_ref().and_then(|r| r.exceptions), &|v| format!("{v:.0} /s"))),
    ];
    let painter = ui.painter();
    painter.rect(stats_rect, 6.0, Color32::TRANSPARENT, Stroke::new(1.0, p.island_border), egui::StrokeKind::Inside);
    if stats_w > 0.0 {
        painter.text(stats_rect.left_top() + vec2(10.0, 8.0), Align2::LEFT_TOP, tr(".NET runtime"), theme::bold(12.0), p.text);
        for (i, (k, v)) in rows.iter().enumerate() {
            let y = stats_rect.top() + 34.0 + i as f32 * 22.0;
            painter.text(pos2(stats_rect.left() + 10.0, y), Align2::LEFT_TOP, *k, FontId::proportional(12.0), p.muted);
            painter.text(pos2(stats_rect.right() - 10.0, y), Align2::RIGHT_TOP, v, FontId::monospace(12.0), p.text);
        }
        if rt.is_none() {
            let g = painter.layout(tr("The runtime's counters show up when the process opens its diagnostic port.").to_string(), FontId::proportional(11.5), p.muted, stats_w - 20.0);
            painter.galley(pos2(stats_rect.left() + 10.0, stats_rect.top() + 34.0 + rows.len() as f32 * 22.0 + 8.0), g, p.muted);
        }
    } else {
        // Narrow: three columns of two.
        let col_w = stats_rect.width() / 3.0;
        for (i, (k, v)) in rows.iter().enumerate() {
            let x = stats_rect.left() + 10.0 + (i % 3) as f32 * col_w;
            let y = stats_rect.top() + 10.0 + (i / 3) as f32 * 32.0;
            painter.text(pos2(x, y), Align2::LEFT_TOP, *k, FontId::proportional(11.0), p.muted);
            painter.text(pos2(x, y + 14.0), Align2::LEFT_TOP, v, FontId::monospace(12.0), p.text);
        }
    }
}

fn gc_heap_series<'a>(samples: impl Iterator<Item = &'a Sample>) -> Vec<(f64, f64)> {
    samples.filter_map(|s| Some((s.t, s.runtime.as_ref()?.gc_heap_mb?))).collect()
}

fn bytes(v: f64) -> String {
    if v >= 1024.0 * 1024.0 {
        format!("{:.1} MB", v / (1024.0 * 1024.0))
    } else if v >= 1024.0 {
        format!("{:.0} KB", v / 1024.0)
    } else {
        format!("{v:.0} B")
    }
}

/// A line chart of the last [`WINDOW`] seconds: title and current value on top, a legend
/// when there are several series, grid lines with their values.
fn chart(ui: &Ui, rect: Rect, title: &str, value: &str, series: &[Series], max: Option<f64>, unit: &str) {
    let p = rc_kit::pal(ui);
    let painter = ui.painter_at(rect);
    painter.rect(rect, 6.0, Color32::TRANSPARENT, Stroke::new(1.0, p.island_border), egui::StrokeKind::Inside);
    painter.text(rect.left_top() + vec2(10.0, 8.0), Align2::LEFT_TOP, title, theme::bold(12.0), p.text);
    painter.text(rect.right_top() + vec2(-10.0, 8.0), Align2::RIGHT_TOP, value, FontId::monospace(12.0), p.text);
    if series.len() > 1 {
        let mut x = rect.left() + 10.0;
        for (_, color, name) in series {
            painter.rect_filled(Rect::from_min_size(pos2(x, rect.top() + 30.0), vec2(8.0, 8.0)), 2.0, *color);
            let g = painter.layout_no_wrap(name.to_string(), FontId::proportional(11.0), p.muted);
            let w = g.size().x;
            painter.galley(pos2(x + 12.0, rect.top() + 27.0), g, p.muted);
            x += 12.0 + w + 14.0;
        }
    }
    let plot = Rect::from_min_max(pos2(rect.left() + 10.0, rect.top() + if series.len() > 1 { 46.0 } else { 32.0 }), pos2(rect.right() - 44.0, rect.bottom() - 10.0));
    if plot.height() < 20.0 {
        return;
    }
    let t_end = series.iter().filter_map(|(s, _, _)| s.last().map(|v| v.0)).fold(0.0, f64::max).max(WINDOW);
    let t_start = t_end - WINDOW;
    let peak = series.iter().flat_map(|(s, _, _)| s.iter().filter(|v| v.0 >= t_start).map(|v| v.1)).fold(0.0, f64::max);
    let top = max.unwrap_or_else(|| nice_ceiling(peak * 1.15)).max(1.0);
    for i in 0..=2 {
        let v = top * i as f64 / 2.0;
        let y = plot.bottom() - plot.height() * (i as f32 / 2.0);
        painter.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.0, p.grid));
        painter.text(pos2(plot.right() + 6.0, y), Align2::LEFT_CENTER, format!("{v:.0} {unit}"), FontId::proportional(10.5), p.muted);
    }
    let to_pos = |(t, v): (f64, f64)| pos2(plot.left() + ((t - t_start) / WINDOW) as f32 * plot.width(), plot.bottom() - (v / top).clamp(0.0, 1.0) as f32 * plot.height());
    for (s, color, _) in series {
        let pts: Vec<egui::Pos2> = s.iter().filter(|v| v.0 >= t_start).map(|&v| to_pos(v)).collect();
        if pts.len() >= 2 {
            // The area under the line, one quad per segment (the whole shape is not convex).
            let mut mesh = egui::Mesh::default();
            let fill = color.gamma_multiply(0.12);
            for w in pts.windows(2) {
                let i = mesh.vertices.len() as u32;
                for v in [w[0], w[1], pos2(w[1].x, plot.bottom()), pos2(w[0].x, plot.bottom())] {
                    mesh.colored_vertex(v, fill);
                }
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
            }
            painter.add(mesh);
            painter.add(egui::Shape::line(pts, Stroke::new(1.5, *color)));
        }
    }
}

/// 1, 2 or 5 times a power of ten, at least `v`.
fn nice_ceiling(v: f64) -> f64 {
    if v <= 0.0 {
        return 1.0;
    }
    let mag = 10f64.powf(v.log10().floor());
    [1.0, 2.0, 5.0, 10.0].into_iter().map(|f| f * mag).find(|&n| n >= v).unwrap_or(10.0 * mag)
}

#[cfg(test)]
mod tests {
    #[test]
    fn nice_ceilings() {
        assert_eq!(super::nice_ceiling(73.0), 100.0);
        assert_eq!(super::nice_ceiling(130.0), 200.0);
        assert_eq!(super::nice_ceiling(0.3), 0.5);
        assert_eq!(super::bytes(3.5 * 1024.0 * 1024.0), "3.5 MB");
    }
}
