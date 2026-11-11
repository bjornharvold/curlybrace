//! The Shell tab's output modes: the `Text | Table | Tree` switch, and for Table and Tree either
//! the view or, when that mode cannot show the run's value, the reason in words with a one-click
//! way back to Text. The notes that say what is not shown (more in the cursor, part of a long
//! array) are plain labels, so they are announced. Only draws and reports actions.
use super::shell_result_table::show_result_table;
use super::shell_result_tree::show_result_tree;
use crate::I18n;
use crate::app::shell_actions::ShellAction;
use crate::theme::{segmented_hinted, text_button};
use crate::viewmodel::index_detail::size_parts;
use crate::viewmodel::shell::ShellState;
use crate::viewmodel::shell_result::{Availability, OutputMode, Reason, ShellResult};
use curlybrace_core::shell::DocKind;
use eframe::egui::{self, accesskit};
use uuid::Uuid;

fn mode_key(mode: OutputMode) -> &'static str {
    match mode {
        OutputMode::Text => "shell.result.mode.text",
        OutputMode::Table => "shell.result.mode.table",
        OutputMode::Tree => "shell.result.mode.tree",
    }
}

/// "Show as" and the three modes, each a radio button with its selected state; a mode that
/// cannot show the value is disabled and says why on hover.
pub(crate) fn show_mode_switch(
    ui: &mut egui::Ui,
    state: &ShellState,
    i18n: &I18n,
) -> Option<ShellAction> {
    let items: Vec<(OutputMode, String)> = OutputMode::ALL
        .into_iter()
        .map(|mode| (mode, i18n.t(mode_key(mode))))
        .collect();
    let available = |mode: &OutputMode| state.availability(*mode) == Availability::Available;
    let hint = |mode: &OutputMode| match state.availability(*mode) {
        Availability::Available => None,
        Availability::Unavailable(reason) => Some(reason_text(reason, i18n)),
    };
    let name = i18n.t("shell.result.mode_label");
    ui.label(name.as_str());
    // The three options sit in a radio group named like the visible label.
    let ctx = ui.ctx().clone();
    let group = egui::Id::new("shell_output_mode_group");
    ctx.accesskit_node_builder(group, |node| {
        node.set_role(accesskit::Role::RadioGroup);
        node.set_label(name);
    });
    ctx.with_accessibility_parent(group, || {
        segmented_hinted(ui, &items, state.mode, available, hint)
    })
    .map(ShellAction::SetOutputMode)
}

/// The Table or Tree of the run's value, or why there is none. Text mode draws nothing here.
pub(crate) fn show_result(
    ui: &mut egui::Ui,
    state: &ShellState,
    session: Uuid,
    i18n: &I18n,
) -> Option<ShellAction> {
    match (state.mode, state.availability(state.mode)) {
        (OutputMode::Text, _) => None,
        (_, Availability::Unavailable(reason)) => show_unavailable(ui, reason, i18n),
        (OutputMode::Table, Availability::Available) => {
            show_result_table(ui, state, session, i18n);
            None
        }
        (OutputMode::Tree, Availability::Available) => show_result_tree(ui, state, session, i18n),
    }
}

fn show_unavailable(ui: &mut egui::Ui, reason: Reason, i18n: &I18n) -> Option<ShellAction> {
    ui.label(reason_text(reason, i18n));
    text_button(ui, i18n.t("shell.result.show_text"))
        .clicked()
        .then_some(ShellAction::SetOutputMode(OutputMode::Text))
}

/// Why a mode cannot show the value, in words.
pub(crate) fn reason_text(reason: Reason, i18n: &I18n) -> String {
    let key = match reason {
        Reason::Pending => "shell.result.unavailable.pending",
        Reason::NoValue => "shell.result.unavailable.no_value",
        Reason::NotDocuments => "shell.result.unavailable.not_documents",
        Reason::Unsupported => "shell.result.unavailable.unsupported",
        Reason::RunFailed => "shell.result.unavailable.run_failed",
        Reason::TooBig(bytes) => {
            let (unit, n) = size_parts(Some(bytes));
            let size = i18n.t_with(unit, &[("n", &n)]);
            return i18n.t_with("shell.result.unavailable.too_big", &[("size", &size)]);
        }
    };
    i18n.t(key)
}

/// What the value leaves out: the rest of a cursor, or the rest of a long array.
pub(super) fn show_notes(ui: &mut egui::Ui, result: &ShellResult, i18n: &I18n) {
    if let Some(note) = note_text(result, i18n) {
        ui.label(note);
    }
}

fn note_text(result: &ShellResult, i18n: &I18n) -> Option<String> {
    let showing_of = |shown: usize, total: u64| {
        let (shown, total) = (shown.to_string(), total.to_string());
        let params = [("shown", shown.as_str()), ("total", total.as_str())];
        i18n.t_with("shell.result.showing_of", &params)
    };
    match result {
        ShellResult::Documents {
            kind: DocKind::Cursor,
            docs,
            more: true,
            ..
        } => {
            let count = docs.len().to_string();
            Some(i18n.t_with("shell.result.more_in_cursor", &[("count", &count)]))
        }
        ShellResult::Documents {
            docs,
            total: Some(total),
            ..
        } if *total > docs.len() as u64 => Some(showing_of(docs.len(), *total)),
        ShellResult::Values { values, total, .. } if *total > values.len() as u64 => {
            Some(showing_of(values.len(), *total))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::{Bson, doc};

    #[test]
    fn a_short_array_and_a_finished_cursor_have_no_note() {
        let i18n = I18n::english();
        let array = ShellResult::Documents {
            kind: DocKind::Array,
            docs: vec![doc! {}],
            more: false,
            total: Some(1),
        };
        let cursor = ShellResult::Documents {
            kind: DocKind::Cursor,
            docs: vec![doc! {}],
            more: false,
            total: None,
        };
        let values = ShellResult::Values {
            values: vec![Bson::Int32(1)],
            more: false,
            total: 1,
        };
        for result in [array, cursor, values] {
            assert_eq!(note_text(&result, &i18n), None, "{result:?}");
        }
    }

    #[test]
    fn a_long_array_of_values_says_how_many_it_shows() {
        let i18n = I18n::english();
        let values = ShellResult::Values {
            values: vec![Bson::Int32(1); 200],
            more: true,
            total: 300,
        };
        let expected = i18n.t_with(
            "shell.result.showing_of",
            &[("shown", "200"), ("total", "300")],
        );
        assert_eq!(note_text(&values, &i18n), Some(expected));
    }
}
