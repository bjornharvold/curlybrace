//! JSON view: virtualised monospace lines of the cached relaxed extended JSON.
//!
//! Ruling: lines are virtualised (`show_rows`) instead of one `TextEdit`, because laying out
//! a 10,000-element document as a single text widget every frame is too slow. Lines keep
//! their natural width inside a two-axis scroll area so long values stay readable.
use super::context_menu::row_context_menu;
use super::metrics::{fixed_row_sized, row_height};
use crate::I18n;
use crate::viewmodel::{DocAction, MenuItem, PageKey, TreeState, menu_items};
use curlybrace_core::store::Page;
use eframe::egui;

pub fn show_json(
    ui: &mut egui::Ui,
    page: &Page,
    key: &PageKey,
    state: &mut TreeState,
    i18n: &I18n,
) -> Option<DocAction> {
    let (_, chosen) = render_json(ui, page, key, state, i18n);
    let (doc_index, item) = chosen?;
    let doc = page.docs.get(doc_index)?;
    let (next, action) = std::mem::take(state).menu_chosen(item, doc, key);
    *state = next;
    action
}

type Chosen = Option<(usize, MenuItem)>;

/// Draws the lines; returns the scroll area output (for size checks) and any menu choice.
pub(crate) fn render_json(
    ui: &mut egui::Ui,
    page: &Page,
    key: &PageKey,
    state: &mut TreeState,
    i18n: &I18n,
) -> (egui::scroll_area::ScrollAreaOutput<()>, Chosen) {
    let lines = state.json_cache(page, key).shared_lines();
    let height = row_height(ui);
    let projected = state.projected;
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let color = ui.visuals().text_color();
    let mut chosen: Chosen = None;
    let output = egui::ScrollArea::both()
        .id_salt("json_view")
        .auto_shrink([false; 2])
        .show_rows(ui, height, lines.len(), |ui, range| {
            for line in &lines[range] {
                let Some(doc) = page.docs.get(line.doc_index) else {
                    continue;
                };
                let text_width = ui.fonts(|f| {
                    f.layout_no_wrap(line.text.clone(), font.clone(), color)
                        .size()
                        .x
                });
                let width = text_width.max(ui.available_width());
                let (_, response) = fixed_row_sized(ui, height, width, |ui| {
                    let text = egui::RichText::new(&line.text).monospace();
                    let label = egui::Label::new(text)
                        .wrap_mode(egui::TextWrapMode::Extend)
                        .selectable(true)
                        .sense(egui::Sense::click());
                    ui.add(label)
                });
                if let Some(item) = row_context_menu(&response, &menu_items(doc, projected), i18n) {
                    chosen = Some((line.doc_index, item));
                }
            }
        });
    (output, chosen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewmodel::page_key;
    use bson::doc;

    const LONG_VALUE_CHARS: usize = 5_000;
    const VIEWPORT_WIDTH: f32 = 300.0;

    #[test]
    fn long_lines_widen_the_scroll_content_beyond_the_viewport() {
        let page = Page {
            docs: vec![doc! { "_id": 1_i32, "text": "x".repeat(LONG_VALUE_CHARS) }],
            skip: 0,
            has_more: false,
        };
        let i18n = I18n::english();
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(VIEWPORT_WIDTH, 400.0),
            )),
            ..Default::default()
        };
        let mut state = TreeState::default();
        let mut widths = None;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let (out, chosen) = render_json(ui, &page, &page_key(&page), &mut state, &i18n);
                assert!(chosen.is_none());
                widths = Some((out.content_size.x, out.inner_rect.width()));
            });
        });
        let (content, viewport) = widths.expect("frame ran");
        assert!(content > viewport, "content {content} viewport {viewport}");
    }
}
