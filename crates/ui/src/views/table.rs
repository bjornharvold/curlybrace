//! Table view: one column per field name, one virtualised row per document.
use super::context_menu::row_context_menu;
use super::doc_select::{apply_toggle, checkbox_width, doc_checkbox, resolve_choice};
use super::metrics::row_height;
use crate::I18n;
use crate::theme::{Scheme, StateLayer, card, current_ui, state_layer};
use crate::viewmodel::{DocAction, PageKey, TreeState, cell_text, menu_items, table_columns};
use curlybrace_core::store::Page;
use eframe::egui;
use egui_extras::{Column, TableBuilder};

const COLUMN_INITIAL_WIDTH: f32 = 140.0;

/// The table: a checkbox column, then one resizable column per field.
fn table_builder(ui: &mut egui::Ui, fields: usize, check_width: f32) -> TableBuilder<'_> {
    let builder = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .vscroll(true)
        .column(Column::exact(check_width));
    (0..fields).fold(builder, |builder, _| {
        builder.column(Column::initial(COLUMN_INITIAL_WIDTH).clip(true))
    })
}

pub fn show_table(
    ui: &mut egui::Ui,
    page: &Page,
    key: &PageKey,
    state: &mut TreeState,
    i18n: &I18n,
) -> Option<DocAction> {
    let columns = table_columns(&page.docs);
    if columns.is_empty() {
        return None;
    }
    let height = row_height(ui);
    let projected = state.projected;
    let read_only = state.read_only;
    let mut chosen = None;
    let mut toggled = None;
    let check_width = checkbox_width(ui);
    // The card is the table's frame; the layout inside it is the plain table.
    card(ui, |ui| {
        let s = current_ui(ui);
        // egui_extras fills a hovered row with `widgets.hovered.bg_fill`; here that is the hover
        // layer over the card's own surface instead of over the stripe color.
        ui.visuals_mut().widgets.hovered.bg_fill =
            state_layer(s.surface_container_low, s.on_surface, StateLayer::Hover);
        // Where the table sits inside the card; the header band is never painted outside of it.
        let viewport = ui.available_rect_before_wrap().intersect(ui.clip_rect());
        table_builder(ui, columns.len(), check_width)
            .header(height, |header| header_cells(header, &columns, s, viewport))
            .body(|body| {
                body.rows(height, page.docs.len(), |mut row| {
                    let index = row.index();
                    let Some(doc) = page.docs.get(index) else {
                        return;
                    };
                    row.col(|ui| {
                        if doc_checkbox(ui, state, page, index, i18n) {
                            toggled = Some(index);
                        }
                    });
                    for name in &columns {
                        row.col(|ui| cell(ui, doc, name, i18n));
                    }
                    let items = menu_items(doc, projected);
                    let copy = state.copy_entry_for(doc);
                    let response = row.response();
                    if let Some(item) = row_context_menu(&response, &items, read_only, copy, i18n) {
                        chosen = Some((index, item));
                    }
                });
            });
    });
    apply_toggle(state, page, toggled);
    resolve_choice(state, page, key, chosen)
}

fn header_cells(
    mut header: egui_extras::TableRow<'_, '_>,
    columns: &[String],
    s: &Scheme,
    viewport: egui::Rect,
) {
    header.col(|ui| paint_header_cell(ui, s, viewport));
    for name in columns {
        header.col(|ui| {
            paint_header_cell(ui, s, viewport);
            ui.strong(name);
        });
    }
}

/// The header row's tonal fill behind one cell, widened by half the spacing on each side so the
/// cells join into one band. A cell's own clip rectangle would cut that widening, so the band is
/// clipped to the table's `viewport` instead: a cell scrolled out of view never paints over the
/// card's padding or the panels around it.
pub(super) fn paint_header_cell(ui: &egui::Ui, s: &Scheme, viewport: egui::Rect) {
    let band = ui.max_rect().expand2(0.5 * ui.spacing().item_spacing);
    // `set_clip_rect` replaces the cell's clip rect (`with_clip_rect` would intersect with it).
    let mut painter = ui.painter().clone();
    painter.set_clip_rect(viewport);
    painter.rect_filled(band, 0.0, s.surface_container_high);
}

fn cell(ui: &mut egui::Ui, doc: &bson::Document, name: &str, i18n: &I18n) {
    let text = doc
        .get(name)
        .map(|b| cell_text(b, i18n))
        .unwrap_or_default();
    ui.add(egui::Label::new(text).truncate());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{self, StateLayer, state_layer};
    use crate::viewmodel::page_key;
    use bson::doc;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// Distance right of a short cell text that is still inside its cell.
    const CELL_MARGIN: f32 = 40.0;

    struct Pane {
        page: Page,
        state: TreeState,
    }

    fn open() -> Harness<'static, Pane> {
        let page = Page {
            docs: vec![
                doc! { "_id": 1_i32, "name": "Ada" },
                doc! { "_id": 2_i32, "name": "Grace" },
                doc! { "_id": 3_i32, "name": "Edsger" },
            ],
            skip: 0,
            has_more: false,
        };
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 300.0))
            .build_ui_state(
                |ui, pane: &mut Pane| {
                    let key = page_key(&pane.page);
                    pane.state.synced_to(&key);
                    show_table(ui, &pane.page, &key, &mut pane.state, &I18n::english());
                },
                Pane {
                    page,
                    state: TreeState::default(),
                },
            );
        theme::install(&harness.ctx);
        harness.run_steps(4);
        harness
    }

    fn fills(harness: &Harness<'_, Pane>) -> Vec<(egui::Rect, egui::Color32)> {
        harness
            .output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(r) => Some((r.rect, r.fill)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_header_row_is_painted_with_surface_container_high() {
        let harness = open();
        let s = theme::current(&harness.ctx);
        let name = harness.get_by_label("name").rect();
        let painted = fills(&harness)
            .into_iter()
            .any(|(rect, fill)| fill == s.surface_container_high && rect.contains_rect(name));
        assert!(painted, "no surface_container_high behind the header cell");
    }

    #[test]
    fn the_header_band_of_a_wide_table_stays_inside_the_card() {
        let page = Page {
            docs: vec![bson::Document::from_iter(
                (0..12).map(|i| (format!("field_{i}"), bson::Bson::Int32(i))),
            )],
            skip: 0,
            has_more: false,
        };
        let mut harness = Harness::builder()
            .with_size(egui::vec2(360.0, 200.0))
            .build_ui_state(
                |ui, pane: &mut Pane| {
                    let key = page_key(&pane.page);
                    egui::Frame::new().inner_margin(24).show(ui, |ui| {
                        show_table(ui, &pane.page, &key, &mut pane.state, &I18n::english());
                    });
                },
                Pane {
                    page,
                    state: TreeState::default(),
                },
            );
        theme::install(&harness.ctx);
        harness.run_steps(4);
        let s = theme::current(&harness.ctx);
        let screen = harness.ctx.screen_rect();
        let bands: Vec<egui::Rect> = harness
            .output()
            .shapes
            .iter()
            .filter(|clipped| {
                matches!(&clipped.shape, egui::Shape::Rect(r) if r.fill == s.surface_container_high)
            })
            .map(|clipped| clipped.clip_rect)
            .collect();
        assert!(!bands.is_empty(), "no header band was painted");
        for clip in bands {
            assert!(
                clip.left() >= screen.left() + 24.0 && clip.right() <= screen.right() - 24.0,
                "a header band is clipped to {clip:?}, wider than the table's card"
            );
        }
    }

    #[test]
    fn a_hovered_row_is_painted_with_the_hover_state_layer() {
        let mut harness = open();
        let s = theme::current(&harness.ctx);
        let hover = state_layer(s.surface_container_low, s.on_surface, StateLayer::Hover);
        assert!(
            fills(&harness).iter().all(|(_, fill)| *fill != hover),
            "a row is hovered before the pointer arrived"
        );
        let grace = harness.get_by_label("Grace").rect();
        // Over the empty part of the cell: text on top of it takes the hover from the row.
        let pointer = egui::pos2(grace.right() + CELL_MARGIN, grace.center().y);
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(pointer));
        harness.run();
        let painted = fills(&harness)
            .into_iter()
            .any(|(rect, fill)| fill == hover && rect.contains_rect(grace));
        assert!(painted, "the hovered row is not painted with {hover:?}");
    }
}
