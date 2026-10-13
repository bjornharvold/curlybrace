//! The preflight strip: the Check button, what the last check found, and what blocks a run.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{CompareState, PreflightView};
use curlybrace_core::compare::protocol::Preflight;
use curlybrace_core::store::IndexStatus;
use eframe::egui;

/// `setup_ready`: the setup has no issues (computed once per frame by the caller). Until the
/// pair list has views of its own, the button first adds the picked pair, then checks the list.
pub fn show_preflight(
    ui: &mut egui::Ui,
    state: &CompareState,
    setup_ready: bool,
    i18n: &I18n,
) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        // While a check is in flight another cannot start, and an edit meanwhile leaves the
        // answer stale: the button says so instead of silently doing nothing.
        let label = if state.check_in_flight() {
            "compare.preflight.checking"
        } else {
            "compare.preflight.check"
        };
        let button = egui::Button::new(i18n.t(label));
        let adding = state.pairs.entries.is_empty();
        let enabled = if adding {
            setup_ready && state.is_ready()
        } else {
            state.can_check()
        };
        if ui.add_enabled(enabled, button).clicked() {
            action = Some(if adding {
                CompareAction::AddPair
            } else {
                CompareAction::RunCheck
            });
        }
        if state.check_in_flight() {
            ui.spinner();
        }
        // While checking, the button text and the spinner already say so.
        if let Some(key) = status_key(&state.preflight_view()) {
            ui.label(i18n.t(key));
        }
    });
    show_sizes(ui, state.last_check(), i18n);
    show_message(ui, &state.preflight_view(), i18n);
    action
}

fn status_key(preflight: &PreflightView) -> Option<&'static str> {
    match preflight {
        PreflightView::Unchecked => Some("compare.preflight.unchecked"),
        PreflightView::Checking => None,
        PreflightView::Ok => Some("compare.preflight.ok"),
        PreflightView::NeedsIndex { .. } => Some("compare.preflight.blocked"),
        PreflightView::EmptyBoth => Some("compare.preflight.empty"),
    }
}

/// One line per side: how many documents it has and whether the key is indexed.
fn show_sizes(ui: &mut egui::Ui, check: Option<&Preflight>, i18n: &I18n) {
    let Some(check) = check else {
        return;
    };
    let sides = [
        (
            "compare.setup.source_heading",
            check.source_count,
            check.source_index,
        ),
        (
            "compare.setup.target_heading",
            check.target_count,
            check.target_index,
        ),
    ];
    for (heading, count, index) in sides {
        let index = i18n.t(index_key(index));
        let line = i18n.t_with(
            "compare.preflight.side_summary",
            &[
                ("side", &i18n.t(heading)),
                ("count", &count.to_string()),
                ("index", &index),
            ],
        );
        ui.label(line);
    }
}

fn index_key(status: IndexStatus) -> &'static str {
    match status {
        IndexStatus::Indexed => "compare.preflight.index_indexed",
        IndexStatus::NotIndexed => "compare.preflight.index_missing",
    }
}

/// The explanation with its next step, in the error colour when it blocks the run.
fn show_message(ui: &mut egui::Ui, preflight: &PreflightView, i18n: &I18n) {
    let Some(message) = preflight.message() else {
        return;
    };
    let text = match message.side_key {
        Some(side) => i18n.t_with(message.key, &[("side", &i18n.t(side))]),
        None => i18n.t(message.key),
    };
    if preflight.allows_start() {
        ui.label(text);
    } else {
        let color = ui.visuals().error_fg_color;
        ui.colored_label(color, text);
    }
}
