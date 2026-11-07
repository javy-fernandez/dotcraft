//! Monitoring tool window: CPU and memory charts of the running program and the .NET runtime's
//! counters, like Rider's Monitoring.

use crate::app::DotCraftApp;
use crate::monitor::{Monitor, Sample};
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
    if let Some(pid) = m.app_pid.or(m.pid) {
        let state = if m.running { trf("{name} · PID {pid}", &[("name", &m.name), ("pid", &pid)]) } else { trf("{name} · exited", &[("name", &m.name)]) };
        ui.painter().text(pos2(title.right() + 12.0, r.center().y), Align2::LEFT_CENTER, state, FontId::proportional(12.0), p.muted);
    }
    let br = Rect::from_min_size(pos2(r.right() - 8.0 - 26.0, r.center().y - 13.0), vec2(26.0, 26.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
    if theme::icon_button(&mut child, rc_kit::ico::HIDE, true, tr("Hide")).clicked() {
        app.toggle_tool(ToolWindow::Monitoring);
        return;
    }
    if m.samples.is_empty() {
        empty_state(ui, ToolWindow::Monitoring.icon(), tr("Nothing to monitor"), tr("Run a project to see its CPU, memory and garbage collections here."));
        return;
    }
    charts(ui, m);
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
