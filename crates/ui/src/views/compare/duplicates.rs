//! Find duplicate keys: the button under a duplicate-key failure and the panel with what the
//! lookup found. Whether the button is live is `CompareState::can_find_duplicates`.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{CompareState, DuplicatesView};
use curlybrace_core::compare::protocol::DuplicateReport;
use eframe::egui;

/// Nothing is drawn until the lookup is offered or a request exists. Returns the action taken.
pub fn show_duplicates(
    ui: &mut egui::Ui,
    state: &CompareState,
    i18n: &I18n,
) -> Option<CompareAction> {
    let view = &state.duplicates;
    let focus = state.focus_entry();
    let offered = state.can_find_duplicates();
    if !offered && *view == DuplicatesView::Idle {
        return None;
    }
    ui.push_id("compare_duplicates", |ui| {
        ui.add_space(4.0);
        ui.strong(i18n.t("compare.duplicates.title"));
        // One button for the whole panel; its label says Try again once a lookup has failed.
        let key = match view {
            DuplicatesView::Failed { .. } => "compare.duplicates.retry",
            _ => "compare.duplicates.find",
        };
        let button = find_button(ui, offered, key, i18n)
            .and_then(|()| focus.map(CompareAction::FindDuplicates));
        show_body(ui, view, i18n);
        button
    })
    .inner
}

/// The Find / Try again button; disabled whenever the view model does not offer the lookup.
fn find_button(ui: &mut egui::Ui, offered: bool, key: &str, i18n: &I18n) -> Option<()> {
    let button = egui::Button::new(i18n.t(key));
    let clicked = ui.add_enabled(offered, button).clicked();
    clicked.then_some(())
}

fn show_body(ui: &mut egui::Ui, view: &DuplicatesView, i18n: &I18n) {
    match view {
        DuplicatesView::Idle => {}
        DuplicatesView::Loading { stale, .. } => show_loading(ui, *stale, i18n),
        DuplicatesView::Loaded { .. } if view.is_empty_result() => {
            ui.label(i18n.t("compare.duplicates.none_found"));
        }
        DuplicatesView::Loaded { source, target, .. } => {
            show_side(ui, "source", "compare.duplicates.source", source, i18n);
            show_side(ui, "target", "compare.duplicates.target", target, i18n);
            show_hint(ui, i18n);
        }
        DuplicatesView::Failed { .. } => show_hint(ui, i18n),
    }
}

/// A request is out. When the form changed under it, the panel says the old lookup is still
/// finishing (that is why Find is disabled), never a blank panel.
fn show_loading(ui: &mut egui::Ui, stale: bool, i18n: &I18n) {
    let key = if stale {
        "compare.duplicates.stale"
    } else {
        "compare.duplicates.loading"
    };
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(i18n.t(key));
    });
}

/// One side: its heading, its groups (key text and count), the "and more" note or the empty note.
fn show_side(ui: &mut egui::Ui, id: &str, heading: &str, report: &DuplicateReport, i18n: &I18n) {
    ui.push_id(id, |ui| {
        ui.add_space(4.0);
        ui.strong(i18n.t(heading));
        let rows = DuplicatesView::rows(report);
        if rows.is_empty() {
            ui.label(i18n.t("compare.duplicates.none_on_side"));
        }
        for (index, row) in rows.iter().enumerate() {
            ui.push_id(index, |ui| {
                let count = row.count.to_string();
                let text = i18n.t_with(
                    "compare.duplicates.row",
                    &[("key", &row.key_text), ("count", &count)],
                );
                ui.label(egui::RichText::new(text).monospace());
            });
        }
        if report.more {
            ui.label(i18n.t("compare.duplicates.more"));
        }
    });
}

fn show_hint(ui: &mut egui::Ui, i18n: &I18n) {
    ui.add_space(4.0);
    ui.label(i18n.t("compare.duplicates.hint"));
}
