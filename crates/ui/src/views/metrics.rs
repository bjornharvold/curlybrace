//! Layout measurements shared by the virtualised document views.
use eframe::egui;

/// Height of every row in the tree, table and JSON views; the single source of truth
/// for `show_rows` / `TableBody::rows` and for the rows themselves.
pub fn row_height(ui: &egui::Ui) -> f32 {
    ui.spacing().interact_size.y
}

/// Allocates a row of exactly `row_height` and lays `content` out inside it, returning the
/// row's rectangle and the content's result.
pub fn fixed_row<R>(
    ui: &mut egui::Ui,
    row_height: f32,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> (egui::Rect, R) {
    let size = egui::vec2(ui.available_width(), row_height);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    let builder = egui::UiBuilder::new().max_rect(rect).layout(layout);
    let inner = ui.scope_builder(builder, |ui| {
        ui.set_clip_rect(ui.clip_rect().intersect(rect));
        content(ui)
    });
    (rect, inner.inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_exactly_row_height_even_with_taller_content() {
        let ctx = egui::Context::default();
        let mut seen = Vec::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let h = row_height(ui);
                for _ in 0..3 {
                    let (rect, ()) = fixed_row(ui, h, |ui| {
                        ui.label(egui::RichText::new("tall").size(h * 3.0));
                    });
                    seen.push((rect.height(), h));
                }
            });
        });
        assert_eq!(seen.len(), 3);
        for (got, want) in seen {
            assert_eq!(got, want);
        }
    }
}
