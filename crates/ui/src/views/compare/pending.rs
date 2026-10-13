//! The pending-changes panel: the queued writes grouped by pair, each with the place it writes
//! and a remove button, the totals, Review changes… and Clear. Every decision (the grouping,
//! the destinations, why Review is closed) comes from `viewmodel::compare::pending_view`.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{
    ActionBlock, CONFIRM_WORD_KEY, CompareState, Destination, PendingGroup, PendingLine,
};
use crate::views::connections::ConnectionsState;
use curlybrace_core::compare::apply::ApplyKind;
use curlybrace_core::compare::protocol::PairId;
use eframe::egui;
use uuid::Uuid;

/// "connection / database.collection" of a destination.
pub fn destination_text(destination: &Destination, i18n: &I18n) -> String {
    if destination.database.is_empty() && destination.collection.is_empty() {
        return i18n.t("compare.apply.unknown_destination");
    }
    let connection = match destination.connection.is_empty() {
        true => i18n.t("compare.apply.unknown_connection"),
        false => destination.connection.clone(),
    };
    let namespace = format!("{}.{}", destination.database, destination.collection);
    i18n.t_with(
        "compare.apply.destination",
        &[("connection", &connection), ("namespace", &namespace)],
    )
}

/// The "source -> target" names of the pair, or a placeholder when its entry is gone.
pub fn pair_text(state: &CompareState, pair: PairId, i18n: &I18n) -> String {
    match state.pair_labels(pair) {
        Some((source, target)) => i18n.t_with(
            "compare.pairs.pair_label",
            &[("source", &source), ("target", &target)],
        ),
        None => i18n.t("compare.apply.unknown_pair"),
    }
}

pub fn show_pending(
    ui: &mut egui::Ui,
    state: &CompareState,
    connections: &ConnectionsState,
    i18n: &I18n,
) -> Option<CompareAction> {
    if state.pending.is_empty() {
        return None;
    }
    ui.separator();
    ui.strong(i18n.t("compare.apply.pending.title"));
    let name_of = |id: Uuid| {
        connections
            .connections
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.clone())
            .unwrap_or_default()
    };
    let view = state.pending_view(&name_of);
    let busy = state.apply.is_busy();
    let mut action = None;
    for group in &view.groups {
        action = show_group(ui, state, group, busy, i18n).or(action);
    }
    let total = view.documents().to_string();
    ui.label(i18n.t_with("compare.apply.pending.total", &[("count", &total)]));
    show_buttons(ui, state, busy, i18n).or(action)
}

fn show_group(
    ui: &mut egui::Ui,
    state: &CompareState,
    group: &PendingGroup,
    busy: bool,
    i18n: &I18n,
) -> Option<CompareAction> {
    ui.label(egui::RichText::new(pair_text(state, group.pair, i18n)).strong());
    let mut action = None;
    for line in &group.lines {
        let removed = ui
            .push_id(line.index, |ui| show_line(ui, line, busy, i18n))
            .inner;
        action = removed.or(action);
    }
    action
}

/// The line's sentence: what is copied or deleted, where, and how many.
fn line_text(line: &PendingLine, i18n: &I18n) -> String {
    let destination = destination_text(&line.destination, i18n);
    if line.kind == ApplyKind::Fields {
        return i18n.t_with(
            "compare.apply.pending.fields",
            &[
                ("destination", &destination),
                ("fields", &line.fields.to_string()),
                ("documents", &line.documents.to_string()),
            ],
        );
    }
    let key = match (line.kind, line.whole_tab) {
        (ApplyKind::Delete, false) => "compare.apply.pending.delete",
        (ApplyKind::Delete, true) => "compare.apply.pending.delete_all",
        (_, false) => "compare.apply.pending.copy",
        (_, true) => "compare.apply.pending.copy_all",
    };
    i18n.t_with(
        key,
        &[
            ("destination", &destination),
            ("count", &line.documents.to_string()),
        ],
    )
}

fn show_line(
    ui: &mut egui::Ui,
    line: &PendingLine,
    busy: bool,
    i18n: &I18n,
) -> Option<CompareAction> {
    let text = line_text(line, i18n);
    ui.horizontal(|ui| {
        ui.label(&text);
        let remove = ui.add_enabled(
            !busy,
            egui::Button::new(i18n.t("compare.apply.pending.remove")),
        );
        let named = i18n.t_with("compare.apply.pending.remove_named", &[("line", &text)]);
        // The visible text stays short; screen readers hear which item it removes.
        remove.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, !busy, named.clone())
        });
        let remove = match busy {
            true => remove.on_disabled_hover_text(i18n.t(ActionBlock::Busy.i18n_key())),
            false => remove,
        };
        remove
            .clicked()
            .then_some(CompareAction::RemovePending(line.index))
    })
    .inner
}

fn show_buttons(
    ui: &mut egui::Ui,
    state: &CompareState,
    busy: bool,
    i18n: &I18n,
) -> Option<CompareAction> {
    let block = state.review_block();
    let mut action = None;
    ui.horizontal(|ui| {
        let review = egui::Button::new(i18n.t("compare.apply.pending.review"));
        if ui.add_enabled(block.is_none(), review).clicked() {
            action = Some(CompareAction::ReviewApply {
                word: i18n.t(CONFIRM_WORD_KEY),
            });
        }
        let clear = egui::Button::new(i18n.t("compare.apply.pending.clear"));
        if ui.add_enabled(!busy, clear).clicked() {
            action = Some(CompareAction::ClearPending);
        }
        // Behind the review dialog "an apply is in progress" would be misleading.
        let reviewing = state.apply.review().is_some();
        if let Some(reason) = block.filter(|r| !(reviewing && *r == ActionBlock::Busy)) {
            let color = ui.visuals().warn_fg_color;
            ui.colored_label(color, i18n.t(reason.i18n_key()));
        }
    });
    action
}
