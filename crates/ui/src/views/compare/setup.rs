//! The setup panel: the database and collection pickers (they only feed Add pair), and the
//! defaults every pair follows unless it overrides them: key, the two filters and the rules.
use super::rules::show_rules;
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{CompareState, Pick, SideForm};
use curlybrace_core::compare::protocol::Side;
use curlybrace_core::store::CollectionKind;
use eframe::egui;

pub fn show_setup(ui: &mut egui::Ui, state: &CompareState, i18n: &I18n) -> Option<CompareAction> {
    let mut action = None;
    for side in [Side::Source, Side::Target] {
        let shown = show_side(ui, side, state.setup.side(side), i18n);
        action = shown.or(action);
    }
    let add = egui::Button::new(i18n.t("compare.pairs.add_pair"));
    if ui.add_enabled(state.can_add_pair(), add).clicked() {
        action = Some(CompareAction::AddPair);
    }
    ui.separator();
    show_defaults(ui, state, i18n).or(action)
}

/// Key, filters and rules every pair follows, with what is wrong with them (the pickers are not
/// part of the defaults).
fn show_defaults(ui: &mut egui::Ui, state: &CompareState, i18n: &I18n) -> Option<CompareAction> {
    let form = &state.setup;
    let mut action = None;
    ui.strong(i18n.t("compare.pairs.defaults_title"));
    if let Some(text) = text_field(ui, "key", form_key(), &form.key_text, i18n) {
        action = Some(CompareAction::SetKey(text));
    }
    for side in [Side::Source, Side::Target] {
        let shown = show_filter(ui, side, form.side(side), i18n);
        action = shown.or(action);
    }
    if let Some(rules) = show_rules(ui, &form.rules, i18n) {
        action = Some(CompareAction::SetRules(rules));
    }
    for issue in form.default_issues() {
        ui.weak(i18n.t(issue.i18n_key()));
    }
    action
}

fn show_filter(
    ui: &mut egui::Ui,
    side: Side,
    form: &SideForm,
    i18n: &I18n,
) -> Option<CompareAction> {
    ui.push_id(side_id(side), |ui| {
        ui.label(i18n.t(heading_key(side)));
        let filter = ("compare.setup.filter", "compare.setup.filter_hint");
        text_field(ui, "filter", filter, &form.filter_text, i18n)
            .map(|text| CompareAction::SetFilter(side, text))
    })
    .inner
}

/// Caption and hint keys of a text box.
type Keys = (&'static str, &'static str);

fn form_key() -> Keys {
    ("compare.setup.key", "compare.setup.key_hint")
}

/// A labelled single-line box. Returns the new text when the user changed it.
pub(super) fn text_field(
    ui: &mut egui::Ui,
    id: &str,
    keys: Keys,
    value: &str,
    i18n: &I18n,
) -> Option<String> {
    let mut buffer = value.to_string();
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            let caption = ui.label(i18n.t(keys.0));
            let edit = egui::TextEdit::singleline(&mut buffer)
                .hint_text(i18n.t(keys.1))
                .desired_width(f32::INFINITY);
            // The caption is the box's accessible name.
            ui.add(edit).labelled_by(caption.id);
        });
    });
    (buffer != value).then_some(buffer)
}

fn side_id(side: Side) -> &'static str {
    match side {
        Side::Source => "source",
        Side::Target => "target",
    }
}

fn heading_key(side: Side) -> &'static str {
    match side {
        Side::Source => "compare.setup.source_heading",
        Side::Target => "compare.setup.target_heading",
    }
}

fn show_side(ui: &mut egui::Ui, side: Side, form: &SideForm, i18n: &I18n) -> Option<CompareAction> {
    ui.push_id(side_id(side), |ui| {
        ui.strong(i18n.t(heading_key(side)));
        let pick = pickers(ui, form, i18n).map(|p| CompareAction::PickSide(side, p));
        if form.database.is_some() && form.collections.is_empty() {
            return reload_button(ui, side, i18n).or(pick);
        }
        pick
    })
    .inner
}

fn pickers(ui: &mut egui::Ui, form: &SideForm, i18n: &I18n) -> Option<Pick> {
    ui.horizontal(|ui| {
        let db = database_combo(ui, form, i18n).map(Pick::Database);
        collection_combo(ui, form, i18n)
            .map(Pick::Collection)
            .or(db)
    })
    .inner
}

fn reload_button(ui: &mut egui::Ui, side: Side, i18n: &I18n) -> Option<CompareAction> {
    let button = ui.button(i18n.t("compare.setup.reload"));
    let button = button.on_hover_text(i18n.t("compare.setup.reload_hint"));
    button.clicked().then_some(CompareAction::Reload(side))
}

fn database_combo(ui: &mut egui::Ui, form: &SideForm, i18n: &I18n) -> Option<String> {
    let mut picked = None;
    let shown = form
        .database
        .clone()
        .unwrap_or_else(|| i18n.t("compare.setup.pick_database"));
    egui::ComboBox::new("database", i18n.t("compare.setup.database"))
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for name in &form.databases {
                let selected = form.database.as_deref() == Some(name);
                let row = ui.push_id(name, |ui| ui.selectable_label(selected, name));
                if row.inner.clicked() {
                    picked = Some(name.clone());
                }
            }
        });
    picked
}

fn collection_combo(ui: &mut egui::Ui, form: &SideForm, i18n: &I18n) -> Option<String> {
    let mut picked = None;
    let shown = form
        .collection
        .clone()
        .unwrap_or_else(|| i18n.t("compare.setup.pick_collection"));
    let enabled = form.database.is_some();
    ui.add_enabled_ui(enabled, |ui| {
        egui::ComboBox::new("collection", i18n.t("compare.setup.collection"))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (name, kind) in &form.collections {
                    let selected = form.collection.as_deref() == Some(name);
                    let text = collection_text(name, *kind, i18n);
                    let row = ui.push_id(name, |ui| ui.selectable_label(selected, text));
                    if row.inner.clicked() {
                        picked = Some(name.clone());
                    }
                }
            });
    });
    picked
}

/// Views are listed (so the user sees why they cannot be compared) but marked.
fn collection_text(name: &str, kind: CollectionKind, i18n: &I18n) -> String {
    match kind {
        CollectionKind::View => i18n.t_with("compare.setup.view_label", &[("name", name)]),
        _ => name.to_string(),
    }
}
