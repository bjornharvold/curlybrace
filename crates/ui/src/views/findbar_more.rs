//! The find bar's "More" disclosure: its toggle, which says how many of the fields behind it
//! hold a value while it is closed, and Sort / Projection / Skip / Limit on one wrapping row.
//! Whether it is open is decided in `viewmodel::findbar_more` (pure, tested).
use super::findbar::sized_field;
use super::findbar_cell::{field_cell, field_row_height};
use super::findbar_options::{options_toggle_text, show_options_toggle};
use crate::I18n;
use crate::theme::text_button;
use crate::theme::widgets::medium_font;
use curlybrace_core::query::QueryText;
use eframe::egui;

/// Width of the Skip and Limit fields: room for the floating label and a few digits.
const NUMBER_FIELD_WIDTH: f32 = 96.0;
/// Sort and Projection are never narrower than this; below it the row wraps after them and they
/// take half a row each. (Their text scrolls, so 160 is enough for a short stage.)
const JSON_FIELD_MIN_WIDTH: f32 = 160.0;
/// Room left over, so rounding in the layout never wraps the last field of a row.
const ROW_SLACK: f32 = 1.0;
/// The toggle's width until a frame has measured it: "Query options" with no count.
const TOGGLE_GUESS: f32 = 134.0;
/// The most options the toggle's label counts, for reserving its widest form.
const WIDEST_COUNT: usize = 9;

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

/// What the row behind "More" did this frame.
#[derive(Default)]
pub struct MoreOutcome {
    /// Enter was pressed in one of the four fields.
    pub enter: bool,
    /// The Query options toggle was clicked.
    pub toggled: bool,
}

/// Sort and Projection sharing what Skip, Limit and the Query options toggle leave of the row.
/// `options` is whether the section is open and how many options are set.
pub fn show_more_fields(
    ui: &mut egui::Ui,
    text: &mut QueryText,
    (options_open, options_set): (bool, usize),
    i18n: &I18n,
) -> MoreOutcome {
    // The toggle's width, measured the frame before (a change re-runs the frame unseen), as the
    // Filter row measures its buttons.
    let memory = ui.id().with("findbar_more_toggle_width");
    let known = ui.data(|d| d.get_temp::<f32>(memory));
    let height = field_row_height(ui);
    ui.horizontal_wrapped(|ui| {
        ui.set_row_height(height);
        let gap = ui.spacing().item_spacing.x;
        let json = json_width(ui.available_width(), gap, known.unwrap_or(TOGGLE_GUESS));
        let mut enter = false;
        let field = |ui: &mut egui::Ui, width, key, value: &mut String| {
            field_cell(ui, width, height, |ui| {
                sized_field(ui, width, key, value, i18n)
            })
        };
        enter |= field(ui, json, KEY_SORT, &mut text.sort);
        enter |= field(ui, json, KEY_PROJECTION, &mut text.projection);
        enter |= field(ui, NUMBER_FIELD_WIDTH, KEY_SKIP, &mut text.skip);
        enter |= field(ui, NUMBER_FIELD_WIDTH, KEY_LIMIT, &mut text.limit);
        let toggle = show_options_toggle(ui, options_open, options_set, i18n);
        // What the toggle takes in its widest form, whatever the count, so setting an option
        // never changes whether the row wraps.
        let measured = toggle.rect.width() + widening(ui, options_set, i18n);
        if known.is_none_or(|was| (was - measured).abs() > 0.5) {
            ui.data_mut(|d| d.insert_temp(memory, measured));
            ui.ctx().request_discard("find bar options toggle measured");
        }
        MoreOutcome {
            enter,
            toggled: toggle.clicked(),
        }
    })
    .inner
}

/// What the toggle's label would add to its width with the largest count in place of `count`
/// (nothing when it is already the widest), from the button font.
fn widening(ui: &egui::Ui, count: usize, i18n: &I18n) -> f32 {
    let size = egui::TextStyle::Button.resolve(ui.style()).size;
    let color = ui.visuals().text_color();
    let width = |count: usize| {
        let text = options_toggle_text(count, i18n);
        let font = medium_font(ui, size);
        ui.painter().layout_no_wrap(text, font, color).size().x
    };
    (width(WIDEST_COUNT) - width(count)).max(0.0)
}

/// How wide Sort and Projection are: the row holds both, Skip, Limit and the toggle (`toggle`
/// wide) when each of the two keeps at least the minimum, and they share what the others leave.
/// Otherwise the row wraps after Projection and the two take half a row each (a field never
/// goes below the minimum, nor above the row). The halves are floored so rounding never pushes
/// Projection onto a row of its own.
fn json_width(available: f32, gap: f32, toggle: f32) -> f32 {
    let others = 2.0 * NUMBER_FIELD_WIDTH + toggle + 4.0 * gap;
    let shared = ((available - others - ROW_SLACK) / 2.0).floor();
    let width = match shared >= JSON_FIELD_MIN_WIDTH {
        true => shared,
        false => ((available - gap - ROW_SLACK) / 2.0).floor(),
    };
    width.max(JSON_FIELD_MIN_WIDTH).min(available)
}

#[cfg(test)]
#[path = "findbar_more_tests.rs"]
mod tests;
