//! Donut chart drawn straight onto the egui painter (egui has no pie widget).
//! At most 8 slices: anything past the 7th-largest folds into "Other", which
//! keeps every color tellable apart. The full breakdown lives in the table
//! next to the chart.

use eframe::egui::{self, epaint::Mesh, Color32, Pos2, Sense, Stroke, Vec2};
use std::f32::consts::TAU;

/// Validated 8-slot categorical palette (light, dark).
const PALETTE: [(&str, &str); 8] = [
    ("#2a78d6", "#3987e5"), // blue
    ("#eb6834", "#d95926"), // orange
    ("#1baf7a", "#199e70"), // aqua
    ("#eda100", "#c98500"), // yellow
    ("#e87ba4", "#d55181"), // magenta
    ("#008300", "#008300"), // green
    ("#4a3aa7", "#9085e9"), // violet
    ("#e34948", "#e66767"), // red
];
const OTHER: (&str, &str) = ("#a8a69c", "#6f6e68");

pub fn color(slot: usize, dark: bool) -> Color32 {
    let (l, d) = PALETTE.get(slot).copied().unwrap_or(OTHER);
    Color32::from_hex(if dark { d } else { l }).unwrap_or(Color32::GRAY)
}

pub fn other_color(dark: bool) -> Color32 {
    Color32::from_hex(if dark { OTHER.1 } else { OTHER.0 }).unwrap_or(Color32::GRAY)
}

pub struct Slice {
    pub label: String,
    pub value: f64,
    pub color: Color32,
}

/// Fold a sorted-descending list down to 7 named slices + "Other".
pub fn fold(sorted: &[(String, f64)], dark: bool) -> Vec<Slice> {
    let mut out: Vec<Slice> = sorted
        .iter()
        .take(7)
        .enumerate()
        .map(|(i, (l, v))| Slice { label: l.clone(), value: *v, color: color(i, dark) })
        .collect();
    if sorted.len() == 8 {
        let (l, v) = &sorted[7];
        out.push(Slice { label: l.clone(), value: *v, color: color(7, dark) });
    } else if sorted.len() > 8 {
        let rest: f64 = sorted[7..].iter().map(|(_, v)| v).sum();
        out.push(Slice {
            label: format!("Other ({} categories)", sorted.len() - 7),
            value: rest,
            color: other_color(dark),
        });
    }
    out
}

/// Draw the donut; returns the index of the hovered slice, if any.
pub fn donut(ui: &mut egui::Ui, slices: &[Slice], diameter: f32, center_text: &str) -> Option<usize> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::hover());
    let painter = ui.painter_at(rect);
    let c = rect.center();
    let r_out = diameter / 2.0 - 4.0;
    let r_in = r_out * 0.58;
    let total: f64 = slices.iter().map(|s| s.value).sum();
    let surface = ui.visuals().panel_fill;
    let text = ui.visuals().strong_text_color();
    let start = -TAU / 4.0; // 12 o'clock

    if total <= 0.0 {
        painter.circle_stroke(c, (r_out + r_in) / 2.0, Stroke::new(r_out - r_in, ui.visuals().faint_bg_color));
        painter.text(c, egui::Align2::CENTER_CENTER, "No books yet", egui::FontId::proportional(14.0), text);
        return None;
    }

    // Which slice is under the pointer?
    let hovered = resp.hover_pos().and_then(|p| {
        let d = p - c;
        let dist = d.length();
        if dist < r_in || dist > r_out + 4.0 {
            return None;
        }
        let mut a = d.y.atan2(d.x) - start;
        if a < 0.0 {
            a += TAU;
        }
        let mut acc = 0.0;
        slices.iter().position(|s| {
            acc += (s.value / total) as f32 * TAU;
            a <= acc
        })
    });

    let mut a0 = start;
    let mut bounds = Vec::new();
    for (i, s) in slices.iter().enumerate() {
        let sweep = (s.value / total) as f32 * TAU;
        let grow = if hovered == Some(i) { 4.0 } else { 0.0 };
        sector(&painter, c, r_in, r_out + grow, a0, a0 + sweep, s.color);
        bounds.push(a0);
        a0 += sweep;
    }
    // 2px surface-colored gaps between slices
    if slices.len() > 1 {
        for a in bounds {
            let dir = Vec2::angled(a);
            painter.line_segment([c + dir * (r_in - 1.0), c + dir * (r_out + 6.0)], Stroke::new(2.0, surface));
        }
    }

    let (big, small) = match hovered {
        Some(i) => (
            format!("{:.1}%", slices[i].value / total * 100.0),
            slices[i].label.clone(),
        ),
        None => (center_text.to_string(), String::new()),
    };
    painter.text(c - Vec2::new(0.0, if small.is_empty() { 0.0 } else { 9.0 }), egui::Align2::CENTER_CENTER, big, egui::FontId::proportional(22.0), text);
    if !small.is_empty() {
        let short: String = small.chars().take(22).collect();
        painter.text(c + Vec2::new(0.0, 14.0), egui::Align2::CENTER_CENTER, short, egui::FontId::proportional(12.0), ui.visuals().text_color());
    }
    hovered
}

fn sector(p: &egui::Painter, c: Pos2, r_in: f32, r_out: f32, a0: f32, a1: f32, color: Color32) {
    let steps = (((a1 - a0) / TAU) * 180.0).ceil().max(2.0) as u32;
    let mut mesh = Mesh::default();
    for k in 0..=steps {
        let a = a0 + (a1 - a0) * k as f32 / steps as f32;
        let dir = Vec2::angled(a);
        mesh.colored_vertex(c + dir * r_in, color);
        mesh.colored_vertex(c + dir * r_out, color);
    }
    for k in 0..steps {
        let i = k * 2;
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i + 1, i + 3, i + 2);
    }
    p.add(egui::Shape::mesh(mesh));
}
