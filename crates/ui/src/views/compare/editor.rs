//! The editor of one pair's settings: key, the two filters and the rules, each following the
//! defaults until the user types, with Use default to follow them again.
use super::rules::show_rules;
use super::setup::text_field;
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::rows::namespace_label;
use crate::viewmodel::compare::{CompareState, EntryId, PairEntry, PairOverrides, RulesForm};
use curlybrace_core::compare::protocol::Side;
use eframe::egui;

/// The open editor, if a pair is being edited.
pub fn show_editor(ui: &mut egui::Ui, state: &CompareState, i18n: &I18n) -> Option<CompareAction> {
    let entry = state.pairs.editing.and_then(|id| state.pairs.get(id))?;
    let mut action = None;
    ui.push_id(("pair_editor", entry.id), |ui| {
        ui.separator();
        action = show_header(ui, entry, state, i18n);
        let fields = show_fields(ui, entry, state, i18n);
        action = fields.or(action.take());
    });
    action
}

fn show_header(
    ui: &mut egui::Ui,
    entry: &PairEntry,
    state: &CompareState,
    i18n: &I18n,
) -> Option<CompareAction> {
    let pair = i18n.t_with(
        "compare.pairs.pair_label",
        &[
            ("source", &namespace_label(&entry.source)),
            ("target", &namespace_label(&entry.target)),
        ],
    );
    let mut action = None;
    ui.horizontal(|ui| {
        ui.strong(i18n.t_with("compare.pairs.editor_title", &[("pair", &pair)]));
        if ui.button(i18n.t("compare.pairs.editor_done")).clicked() {
            action = Some(CompareAction::EditPair(None));
        }
    });
    if let Some(issue) = state.editor_issue() {
        ui.weak(i18n.t(issue.i18n_key()));
    }
    action
}

fn show_fields(
    ui: &mut egui::Ui,
    entry: &PairEntry,
    state: &CompareState,
    i18n: &I18n,
) -> Option<CompareAction> {
    let o = &entry.overrides;
    let d = &state.setup;
    let key = text_override(ui, "key", KEY_KEYS, &o.key_text, &d.key_text, i18n)
        .map(|v| o.clone().with_key_text(v));
    let filters = [Side::Source, Side::Target].map(|side| {
        let (own, default) = match side {
            Side::Source => (&o.source_filter, &d.source.filter_text),
            Side::Target => (&o.target_filter, &d.target.filter_text),
        };
        let id = if side == Side::Source {
            "source_filter"
        } else {
            "target_filter"
        };
        text_override(ui, id, FILTER_KEYS, own, default, i18n)
            .map(|v| o.clone().with_filter(side, v))
    });
    let rules = rules_override(ui, &o.rules, &d.rules, i18n).map(|v| o.clone().with_rules(v));
    let [source, target] = filters;
    key.or(source)
        .or(target)
        .or(rules)
        .map(|next| set(entry.id, next))
}

const KEY_KEYS: (&str, &str) = ("compare.setup.key", "compare.setup.key_hint");
const FILTER_KEYS: (&str, &str) = ("compare.setup.filter", "compare.setup.filter_hint");

fn set(entry: EntryId, overrides: PairOverrides) -> CompareAction {
    CompareAction::SetPairOverrides(entry, overrides)
}

/// A text field showing the pair's own value, else the default. `Some(new)` when edited
/// (`Some(None)` for Use default).
fn text_override(
    ui: &mut egui::Ui,
    id: &str,
    keys: (&'static str, &'static str),
    own: &Option<String>,
    default: &str,
    i18n: &I18n,
) -> Option<Option<String>> {
    let shown = own.as_deref().unwrap_or(default);
    let mut change = None;
    ui.horizontal(|ui| {
        if let Some(text) = text_field(ui, id, keys, shown, i18n) {
            change = Some(Some(text));
        }
        if use_default_button(ui, own.is_some(), i18n) {
            change = Some(None);
        }
    });
    change
}

fn rules_override(
    ui: &mut egui::Ui,
    own: &Option<RulesForm>,
    default: &RulesForm,
    i18n: &I18n,
) -> Option<Option<RulesForm>> {
    let edited = show_rules(ui, own.as_ref().unwrap_or(default), i18n).map(Some);
    let cleared = use_default_button(ui, own.is_some(), i18n).then_some(None);
    cleared.or(edited)
}

fn use_default_button(ui: &mut egui::Ui, overridden: bool, i18n: &I18n) -> bool {
    let button = egui::Button::new(i18n.t("compare.pairs.use_default"));
    ui.add_enabled(overridden, button).clicked()
}
