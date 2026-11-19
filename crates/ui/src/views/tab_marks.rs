//! What a tab wears besides its title (#159): the connection's colour glyph, compact production
//! and read-only marks, and the soft edges where the strip runs on out of view.
use crate::I18n;
use crate::theme::current_ui;
use crate::viewmodel::ConnectionBadge;
use crate::views::connection_badge::show_badge;
use eframe::egui::{self, Rect, Sense, Stroke, Ui, vec2};

/// The production and read-only marks inside a tab.
pub const MARK_SIZE: f32 = 14.0;

/// The connection's colour glyph as in the header, and production and read-only as short marks
/// (the words are in the tooltip and the accessible name), so a title keeps its room. Shared by
/// the tabs and the All tabs menu: the two never disagree.
pub fn show_marks(ui: &mut Ui, badge: &ConnectionBadge, i18n: &I18n) {
    let s = current_ui(ui);
    let glyph = ConnectionBadge {
        production: false,
        read_only: false,
        ..*badge
    };
    show_badge(ui, &glyph, i18n);
    if badge.production {
        paint_mark(ui, (s.error, s.on_error), Mark::Exclamation);
    }
    if badge.read_only {
        paint_mark(ui, (s.warning, s.on_warning), Mark::Lock);
    }
}

/// The shape inside a compact production / read-only mark.
enum Mark {
    Exclamation,
    Lock,
}

/// A small round mark of `fill` with a `shape` in the `on` colour in it: not text, so nothing to
/// translate. A padlock for read-only reads at 1x where a bare bar did not.
fn paint_mark(ui: &mut Ui, (fill, on): (egui::Color32, egui::Color32), shape: Mark) {
    let size = MARK_SIZE;
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let painter = ui.painter();
    painter.circle_filled(rect.center(), size / 2.0, fill);
    let stroke = Stroke::new(1.5, on);
    let c = rect.center();
    match shape {
        Mark::Exclamation => {
            painter.line_segment([c + vec2(0.0, -4.0), c + vec2(0.0, 0.5)], stroke);
            painter.circle_filled(c + vec2(0.0, 3.5), 1.1, on);
        }
        Mark::Lock => {
            // The body, and the shackle above it.
            let body = Rect::from_center_size(c + vec2(0.0, 1.5), vec2(7.0, 5.0));
            painter.rect_filled(body, 1.0, on);
            painter.line_segment([c + vec2(-2.0, -0.8), c + vec2(-2.0, -3.0)], stroke);
            painter.line_segment([c + vec2(2.0, -0.8), c + vec2(2.0, -3.0)], stroke);
            painter.line_segment([c + vec2(-2.0, -3.0), c + vec2(2.0, -3.0)], stroke);
        }
    }
}

/// A soft edge over the strip where tabs run on out of view, so a tab is never just cut off.
pub fn overflow_fades(ui: &Ui, area: Rect, offset: f32, content: f32) {
    let s = current_ui(ui);
    let clear = s.surface.gamma_multiply(0.0);
    let width = 24.0;
    let edge = |x_range: (f32, f32), from: egui::Color32, to: egui::Color32| {
        let r = Rect::from_x_y_ranges(x_range.0..=x_range.1, area.y_range());
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(r.left_top(), from);
        mesh.colored_vertex(r.right_top(), to);
        mesh.colored_vertex(r.right_bottom(), to);
        mesh.colored_vertex(r.left_bottom(), from);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        ui.painter().add(egui::Shape::mesh(mesh));
    };
    if offset > 0.5 {
        edge((area.left(), area.left() + width), s.surface, clear);
    }
    if offset + area.width() < content - 0.5 {
        edge((area.right() - width, area.right()), clear, s.surface);
    }
}
