//! The Query options of the find bar: the toggle at the end of the More row, the framed section
//! under it (bounded in height, scrolling inside), and the lines that say which options shaped
//! the shown page. Draws what it is given and returns what the person did; the owner applies the
//! edits and decides what Enter means.
use super::options_form::{FlagsWhenOff, FormContext, FormLayout, show_options_form};
use crate::I18n;
use crate::theme::shape::{self, radius};
use crate::theme::{current_ui, filled_button, tonal_button};
use crate::viewmodel::aggregate_hint_list::HintListUi;
use crate::viewmodel::aggregate_options::OptionsEdit;
use crate::viewmodel::findbar::FindBarState;
use crate::viewmodel::findbar_options::{options_open, page_options_status};
use curlybrace_core::pipeline::MaxTimeEffect;
use curlybrace_core::query_options::{FIND_MAX_TIME_MS, FindOperation, QueryOptions};
use eframe::egui::{self, WidgetInfo, WidgetType};

/// The section takes at most this share of the window's height (at least [`OPTIONS_MIN_HEIGHT`]),
/// and scrolls inside, so the first document row stays on screen in a small window.
pub const OPTIONS_MAX_HEIGHT_SHARE: f32 = 0.35;
pub const OPTIONS_MIN_HEIGHT: f32 = 140.0;
/// What the toggle takes of the More row ("Query options (N)" and the gap before it).
pub const TOGGLE_ROW_SHARE: f32 = 170.0;

/// What the section's controls did this frame.
#[derive(Default)]
pub(super) struct SectionOutcome {
    pub edits: Vec<OptionsEdit>,
    pub reset: bool,
    /// Enter was pressed in a text field of the section.
    pub enter: bool,
    /// The section is open and has no index list yet.
    pub wants_hints: bool,
    /// Retry was clicked after the index list could not be read.
    pub retry_hints: bool,
}

/// "Query options" or "Query options (N)" with the accent, named with its expanded state; true
/// when it was clicked.
pub(super) fn show_options_toggle(
    ui: &mut egui::Ui,
    open: bool,
    count: usize,
    i18n: &I18n,
) -> bool {
    let label = match count {
        0 => i18n.t("query.options.toggle"),
        n => i18n.t_with("query.options.toggle_count", &[("n", &n.to_string())]),
    };
    let response = match count {
        0 => tonal_button(ui, label.clone()),
        _ => filled_button(ui, label.clone()),
    }
    .on_hover_text(i18n.t("query.options.toggle_hint"));
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, open, label.clone()));
    response.clicked()
}

/// What the find bar tells the shared form: its own notes and the find's time bound.
fn form_context(state: &FindBarState) -> FormContext<'static> {
    let limit = (FIND_MAX_TIME_MS / 1000).to_string();
    let capped = state.options.parsed().is_ok_and(|options| {
        options.find_max_time_effect(FindOperation::Find) == MaxTimeEffect::Capped
    });
    FormContext {
        id_scope: "find",
        layout: FormLayout::Wrapped,
        disk_use_note: "query.options.disk_use_note",
        comment_placeholder: "query.options.comment_placeholder",
        max_time_placeholder: (
            "query.options.max_time.placeholder",
            vec![("limit", limit.clone())],
        ),
        max_time_clamped: capped
            .then(|| ("query.options.max_time.clamped", vec![("limit", limit)])),
        flags_when_off: FlagsWhenOff::Hidden,
    }
}

fn section_frame(ui: &egui::Ui) -> egui::Frame {
    let s = current_ui(ui);
    egui::Frame::new()
        .fill(s.surface_container_low)
        .stroke(egui::Stroke::new(1.0, s.outline_variant))
        .corner_radius(radius(shape::MEDIUM))
        .inner_margin(egui::Margin::same(shape::SPACE_2 as i8))
        .outer_margin(egui::Margin::symmetric(0, shape::SPACE_1 as i8))
}

/// The section while it is open; nothing while it is closed. It scrolls inside a height of
/// [`OPTIONS_MAX_HEIGHT_SHARE`] of the window.
pub(super) fn show_options_section(
    ui: &mut egui::Ui,
    state: &FindBarState,
    hints: &HintListUi,
    i18n: &I18n,
) -> SectionOutcome {
    if !options_open(state) {
        return SectionOutcome::default();
    }
    let cx = form_context(state);
    let max_height =
        (ui.ctx().screen_rect().height() * OPTIONS_MAX_HEIGHT_SHARE).max(OPTIONS_MIN_HEIGHT);
    let gathered = section_frame(ui)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt("find_options_scroll")
                .max_height(max_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    show_options_form(ui, &state.options, hints, &cx, i18n)
                })
                .inner
        })
        .inner;
    SectionOutcome {
        edits: gathered.edits,
        reset: gathered.reset,
        enter: gathered.enter,
        wants_hints: hints.wants_list(),
        retry_hints: gathered.retry_hints,
    }
}

/// The weak lines after the verdict that say which options shaped the shown page; nothing when
/// none did.
pub fn show_page_options_status(ui: &mut egui::Ui, options: &QueryOptions, i18n: &I18n) {
    for (key, params) in page_options_status(options, i18n) {
        let params: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        ui.weak(i18n.t_with(key, &params));
    }
}

#[cfg(test)]
#[path = "findbar_options_tests.rs"]
mod tests;
