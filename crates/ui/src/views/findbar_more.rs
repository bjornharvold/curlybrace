//! The find bar's "More" disclosure: its toggle, which says how many of the fields behind it
//! hold a value while it is closed, and Sort / Projection / Skip / Limit on one wrapping row.
//! Whether it is open is decided in `viewmodel::findbar_more` (pure, tested).
use super::findbar::sized_field;
use crate::I18n;
use crate::theme::text_button;
use curlybrace_core::query::QueryText;
use eframe::egui;

/// Width of the Skip and Limit fields: room for the floating label and a few digits.
const NUMBER_FIELD_WIDTH: f32 = 96.0;
/// Sort and Projection are never narrower than this; below it the row wraps.
const JSON_FIELD_MIN_WIDTH: f32 = 200.0;

const KEY_SORT: (&str, &str) = ("findbar.sort", "findbar.sort_hint");
const KEY_PROJECTION: (&str, &str) = ("findbar.projection", "findbar.projection_hint");
const KEY_SKIP: (&str, &str) = ("findbar.skip", "");
const KEY_LIMIT: (&str, &str) = ("findbar.limit", "");

/// The toggle: "More options" while open or while nothing is hidden, else it counts the hidden
/// fields that hold a value, so a set Sort is never invisible. Reported as a button with its
/// expanded state.
pub fn show_more_toggle(
    ui: &mut egui::Ui,
    open: bool,
    hidden: usize,
    i18n: &I18n,
) -> egui::Response {
    let label = match (open, hidden) {
        (false, count) if count > 0 => {
            i18n.t_with("findbar.more_set", &[("count", &count.to_string())])
        }
        _ => i18n.t("findbar.more"),
    };
    let response = text_button(ui, label);
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_expanded(open));
    response
}

/// Sort and Projection sharing what Skip and Limit leave of the row; true when Enter was
/// pressed in one of the four.
pub fn show_more_fields(ui: &mut egui::Ui, text: &mut QueryText, i18n: &I18n) -> bool {
    ui.horizontal_wrapped(|ui| {
        let json = json_width(ui.available_width(), ui.spacing().item_spacing.x);
        let mut enter = false;
        enter |= sized_field(ui, json, KEY_SORT, &mut text.sort, i18n);
        enter |= sized_field(ui, json, KEY_PROJECTION, &mut text.projection, i18n);
        enter |= sized_field(ui, NUMBER_FIELD_WIDTH, KEY_SKIP, &mut text.skip, i18n);
        enter |= sized_field(ui, NUMBER_FIELD_WIDTH, KEY_LIMIT, &mut text.limit, i18n);
        enter
    })
    .inner
}

/// Half of what the two number fields and the gaps leave, within the minimum and the row.
fn json_width(available: f32, gap: f32) -> f32 {
    let shared = (available - 2.0 * NUMBER_FIELD_WIDTH - 4.0 * gap) / 2.0;
    shared.max(JSON_FIELD_MIN_WIDTH).min(available)
}
