//! The index builder modal: ordered key rows, the options, the exact command that will be sent,
//! and Create / Cancel. Draws the state it is given and reports what the person did; the reducer
//! decides what happens.
use super::tone_notice::tone_notice;
use crate::I18n;
use crate::theme::{
    FieldOptions, Tone, card, compact_text_button, filled_button, modal_frame, outlined_field_opts,
    text_button, tonal_button, typography,
};
use crate::viewmodel::connection_tag::READ_ONLY_REASON_KEY;
use crate::viewmodel::index_builder::{BUILDER_KINDS, kind_label_key};
use crate::viewmodel::{
    BuildWarning, BuilderEdit, BuilderProblem, IndexBuilderState, IndexesAction,
};
use curlybrace_core::indexes::{IndexProblem, KeyKind};
use curlybrace_core::store::CollectionIndex;
use eframe::egui::{self, WidgetInfo, WidgetType};

const MODAL_WIDTH: f32 = 640.0;
const FIELD_WIDTH: f32 = 220.0;
const NAME_WIDTH: f32 = 300.0;
const TTL_WIDTH: f32 = 240.0;
const JSON_ROWS: usize = 3;
/// The preview grows with the command, between these many lines.
const PREVIEW_ROWS: std::ops::RangeInclusive<usize> = 6..=20;
/// Room kept below the scrolling body for the title, the footer and the modal's own margins.
const CHROME_HEIGHT: f32 = 230.0;

/// A checkbox of the options: its label key, its value and the edit it makes.
type Flag = (&'static str, bool, fn(bool) -> BuilderEdit);

/// What the modal needs besides the builder.
pub struct BuilderContext<'a> {
    /// The label of the connection the index is created on.
    pub connection: &'a str,
    /// The connection refuses writes: Create is disabled and says why.
    pub read_only: bool,
    /// A write is in flight: Create waits for it.
    pub busy: bool,
    /// The indexes the collection has, so a clash with one of them disables Create.
    pub existing: &'a [CollectionIndex],
}

/// Draws the builder as a modal over `ctx`.
pub fn show_index_builder(
    ctx: &egui::Context,
    builder: &IndexBuilderState,
    context: &BuilderContext<'_>,
    i18n: &I18n,
) -> Option<IndexesAction> {
    let mut action = None;
    let max_height = (ctx.screen_rect().height() - CHROME_HEIGHT).max(200.0);
    let modal = egui::Modal::new(egui::Id::new("index_builder"))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(MODAL_WIDTH);
            let namespace = format!("{}.{}", builder.ns.db, builder.ns.collection);
            match builder.replaces() {
                Some(original) => {
                    let title = i18n.t_with("indexes.builder.title_copy", &[("name", original)]);
                    ui.label(typography::dialog_title(ui, title))
                }
                None => ui.label(typography::dialog_title(
                    ui,
                    i18n.t("indexes.builder.title"),
                )),
            };
            ui.label(i18n.t_with(
                "indexes.builder.writing_to",
                &[
                    ("connection", context.connection),
                    ("collection", &namespace),
                ],
            ));
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .auto_shrink([false, true])
                .min_scrolled_height(max_height)
                .show(ui, |ui| {
                    // What was sent cannot be edited while the server works on it.
                    let editable = !builder.submitted;
                    let edit = ui.add_enabled_ui(editable, |ui| body(ui, builder, i18n));
                    action = edit.inner.map(IndexesAction::Builder);
                });
            ui.separator();
            action = footer(ui, builder, context, i18n).or(action.take());
        });
    // Escape or a click outside closes the builder only while it holds nothing to lose.
    let dismissed = modal.should_close() && builder.is_untouched() && !context.busy;
    action.or(dismissed.then_some(IndexesAction::CancelBuilder))
}

fn body(ui: &mut egui::Ui, builder: &IndexBuilderState, i18n: &I18n) -> Option<BuilderEdit> {
    let mut edit = None;
    if let Some(original) = builder.replaces() {
        ui.label(i18n.t_with("indexes.builder.editing_note", &[("name", original)]));
    }
    ui.strong(i18n.t("indexes.builder.keys_heading"));
    edit = key_rows(ui, builder, i18n).or(edit);
    if tonal_button(ui, i18n.t("indexes.builder.add_key")).clicked() {
        edit = Some(BuilderEdit::AddKey);
    }
    if (0..builder.keys.len()).any(|i| builder.is_fixed(i)) {
        ui.weak(i18n.t("indexes.builder.fixed_keys_note"));
    }
    ui.weak(i18n.t("indexes.builder.tip"));
    ui.separator();
    ui.strong(i18n.t("indexes.builder.options_heading"));
    edit = options(ui, builder, i18n).or(edit);
    ui.separator();
    preview(ui, builder, i18n);
    edit
}

fn key_rows(ui: &mut egui::Ui, builder: &IndexBuilderState, i18n: &I18n) -> Option<BuilderEdit> {
    let mut edit = None;
    let last = builder.keys.len().saturating_sub(1);
    let problem = builder.problem();
    for (i, key) in builder.keys.iter().enumerate() {
        let n = (i + 1).to_string();
        let numbered = [("n", n.as_str())];
        // A hashed or 2d key of the original is shown and kept as it is.
        let fixed = builder.is_fixed(i);
        let error = duplicate_error(builder, i, problem.as_ref(), i18n);
        ui.horizontal(|ui| {
            let label = i18n.t_with("indexes.builder.key_field_n", &numbered);
            let mut text = key.field.clone();
            let field = ui
                .add_enabled_ui(!fixed, |ui| {
                    sized_field(ui, FIELD_WIDTH, &label, &mut text, error.as_deref())
                })
                .inner;
            if field.changed() {
                edit = Some(BuilderEdit::SetField(i, text));
            }
            // The controls are centred on the field's box, not on the row, which also holds
            // the floating label above and the reserved error row below the box.
            let at = egui::Rect::from_min_max(
                egui::pos2(ui.cursor().min.x, field.rect.top()),
                egui::pos2(ui.max_rect().right(), field.rect.bottom()),
            );
            let centred = egui::UiBuilder::new()
                .max_rect(at)
                .layout(egui::Layout::left_to_right(egui::Align::Center));
            ui.scope_builder(centred, |ui| {
                if let Some(kind) = kind_combo(ui, i, (&key.kind, !fixed), &numbered, i18n) {
                    edit = Some(BuilderEdit::SetKind(i, kind));
                }
                if row_button(ui, i > 0, &MOVE_UP, &numbered, i18n) {
                    edit = Some(BuilderEdit::MoveKey { from: i, to: i - 1 });
                }
                if row_button(ui, i < last, &MOVE_DOWN, &numbered, i18n) {
                    edit = Some(BuilderEdit::MoveKey { from: i, to: i + 1 });
                }
                if row_button(ui, true, &REMOVE, &numbered, i18n) {
                    edit = Some(BuilderEdit::RemoveKey(i));
                }
            });
        });
    }
    edit
}

/// An outlined field `width` wide that keeps the row of its error free, so an error appearing
/// does not move the dialog.
fn sized_field(
    ui: &mut egui::Ui,
    width: f32,
    label: &str,
    text: &mut String,
    error: Option<&str>,
) -> egui::Response {
    ui.vertical(|ui| {
        ui.set_width(width);
        let options = FieldOptions {
            error,
            reserve_error_row: true,
            ..FieldOptions::default()
        };
        outlined_field_opts(ui, label, text, "", options)
    })
    .inner
}

/// The message under the key row `index` when it repeats the field of an earlier row.
fn duplicate_error(
    builder: &IndexBuilderState,
    index: usize,
    problem: Option<&BuilderProblem>,
    i18n: &I18n,
) -> Option<String> {
    let problem = problem?;
    let BuilderProblem::Index(IndexProblem::DuplicateField(field)) = problem else {
        return None;
    };
    let repeated = builder.keys[index].field == *field
        && builder.keys[..index].iter().any(|k| k.field == *field);
    repeated.then(|| problem_text(problem, i18n))
}

/// A button of a key row: a short visible text and a longer accessible name (which says to which
/// field it applies), both `en.json` keys.
struct RowButton {
    text: &'static str,
    name: &'static str,
}

const MOVE_UP: RowButton = RowButton {
    text: "indexes.builder.move_up",
    name: "indexes.builder.move_up_n",
};
const MOVE_DOWN: RowButton = RowButton {
    text: "indexes.builder.move_down",
    name: "indexes.builder.move_down_n",
};
const REMOVE: RowButton = RowButton {
    text: "indexes.builder.remove_key",
    name: "indexes.builder.remove_key_n",
};

/// Draws `button` for the field in `params`; returns whether it was clicked.
fn row_button(
    ui: &mut egui::Ui,
    enabled: bool,
    button: &RowButton,
    params: &[(&str, &str)],
    i18n: &I18n,
) -> bool {
    let response = ui
        .add_enabled_ui(enabled, |ui| compact_text_button(ui, i18n.t(button.text)))
        .inner;
    let name = i18n.t_with(button.name, params);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, name.clone()));
    response.clicked()
}

/// The key type picker; the kinds the builder does not offer show up only as the current value of
/// a key that already has one.
fn kind_combo(
    ui: &mut egui::Ui,
    index: usize,
    (current, enabled): (&KeyKind, bool),
    params: &[(&str, &str)],
    i18n: &I18n,
) -> Option<KeyKind> {
    let mut chosen = None;
    let shown = i18n.t(kind_label_key(current));
    let combo = ui
        .add_enabled_ui(enabled, |ui| {
            // The picker's own row is as tall as the picker, so it is centred by its centre.
            let height = ui.text_style_height(&egui::TextStyle::Button)
                + 2.0 * ui.spacing().button_padding.y;
            ui.spacing_mut().interact_size.y = ui.spacing().interact_size.y.max(height);
            egui::ComboBox::from_id_salt(("index_key_kind", index))
                .selected_text(shown.clone())
                .show_ui(ui, |ui| {
                    for kind in BUILDER_KINDS {
                        let label = i18n.t(kind_label_key(&kind));
                        if ui.selectable_label(*current == kind, label).clicked() {
                            chosen = Some(kind);
                        }
                    }
                })
        })
        .inner;
    let name = i18n.t_with("indexes.builder.key_type_n", params);
    combo.response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::ComboBox, enabled, name.clone());
        info.current_text_value = Some(shown.clone());
        info
    });
    chosen
}

fn options(ui: &mut egui::Ui, builder: &IndexBuilderState, i18n: &I18n) -> Option<BuilderEdit> {
    let mut edit = None;
    let problem = builder.problem();
    let name_error = match problem {
        Some(BuilderProblem::EmptyName) => problem.as_ref().map(|p| problem_text(p, i18n)),
        _ => None,
    };
    let mut name = builder.name.clone();
    let field = sized_field(
        ui,
        NAME_WIDTH,
        &i18n.t("indexes.builder.name"),
        &mut name,
        name_error.as_deref(),
    );
    if field.changed() {
        edit = Some(BuilderEdit::SetName(name));
    }
    let mut text = builder
        .options
        .expire_after_seconds
        .map(|s| s.to_string())
        .unwrap_or_default();
    let ttl_error = problem
        .as_ref()
        .filter(|p| is_ttl(p))
        .map(|p| problem_text(p, i18n));
    let response = sized_field(
        ui,
        TTL_WIDTH,
        &i18n.t("indexes.builder.ttl"),
        &mut text,
        ttl_error.as_deref(),
    );
    // Text that is not a whole number is not an edit: the box shows the value again.
    if let (true, Some(ttl)) = (response.changed(), parse_seconds(&text)) {
        edit = Some(BuilderEdit::SetTtl(ttl));
    }
    ui.horizontal(|ui| {
        let flags: [Flag; 3] = [
            (
                "indexes.builder.unique",
                builder.options.unique,
                BuilderEdit::SetUnique,
            ),
            (
                "indexes.builder.sparse",
                builder.options.sparse,
                BuilderEdit::SetSparse,
            ),
            (
                "indexes.builder.hidden",
                builder.options.hidden,
                BuilderEdit::SetHidden,
            ),
        ];
        for (key, value, make) in flags {
            let mut on = value;
            if ui.checkbox(&mut on, i18n.t(key)).changed() {
                edit = Some(make(on));
            }
        }
    });
    let error_of = |wanted: &BuilderProblem| {
        problem
            .as_ref()
            .filter(|p| *p == wanted)
            .map(|p| problem_text(p, i18n))
    };
    let collation = json_box(
        ui,
        (
            &i18n.t("indexes.builder.collation"),
            &builder.collation_text,
        ),
        error_of(&BuilderProblem::CollationInvalid).as_deref(),
    );
    let partial = json_box(
        ui,
        (
            &i18n.t("indexes.builder.partial_filter"),
            &builder.partial_filter_text,
        ),
        error_of(&BuilderProblem::PartialFilterInvalid).as_deref(),
    );
    collation
        .map(BuilderEdit::SetCollationText)
        .or(partial.map(BuilderEdit::SetPartialFilterText))
        .or(edit)
}

fn is_ttl(problem: &BuilderProblem) -> bool {
    matches!(
        problem,
        BuilderProblem::Index(IndexProblem::TtlNeedsOneOrderedKey | IndexProblem::TtlNegative)
            | BuilderProblem::TtlTooLarge
    )
}

/// `Some(None)` for blank text, `Some(Some(n))` for a whole number, `None` for anything else.
fn parse_seconds(text: &str) -> Option<Option<i64>> {
    match text.trim().is_empty() {
        true => Some(None),
        false => text.trim().parse().ok().map(Some),
    }
}

/// An outlined multi-line JSON box labelled `label`, with the row of its error kept free;
/// returns the new text when it changed.
fn json_box(
    ui: &mut egui::Ui,
    (label, current): (&str, &str),
    error: Option<&str>,
) -> Option<String> {
    let mut text = current.to_string();
    let options = FieldOptions {
        error,
        reserve_error_row: true,
        multiline_rows: Some(JSON_ROWS),
        ..FieldOptions::default()
    };
    let response = outlined_field_opts(ui, label, &mut text, "", options);
    response.changed().then_some(text)
}

/// The command exactly as the store will receive it, read-only but selectable, in a card.
fn preview(ui: &mut egui::Ui, builder: &IndexBuilderState, i18n: &I18n) {
    let caption = ui.strong(i18n.t("indexes.builder.preview_heading"));
    let json = builder.preview_json();
    let mut shown: &str = &json;
    card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(
            egui::TextEdit::multiline(&mut shown)
                .font(egui::TextStyle::Monospace)
                .lock_focus(false)
                .frame(false)
                .desired_rows(
                    json.lines()
                        .count()
                        .clamp(*PREVIEW_ROWS.start(), *PREVIEW_ROWS.end()),
                )
                .desired_width(f32::INFINITY),
        )
        .labelled_by(caption.id);
    });
}

/// The warning and the reason Create is off (if any), then Create and Cancel.
fn footer(
    ui: &mut egui::Ui,
    builder: &IndexBuilderState,
    context: &BuilderContext<'_>,
    i18n: &I18n,
) -> Option<IndexesAction> {
    if let Some(BuildWarning::LargeCollection { documents }) = &builder.warning {
        let text = i18n.t_with(
            "indexes.builder.warning_large",
            &[("documents", &documents.to_string())],
        );
        tone_notice(ui, Tone::Warning, &text, false);
    }
    let problem = builder.problem_among(context.existing);
    if context.read_only {
        ui.label(i18n.t(READ_ONLY_REASON_KEY));
    }
    if context.busy {
        ui.label(i18n.t("indexes.builder.busy"));
    }
    let mut action = None;
    ui.horizontal(|ui| {
        let enabled = problem.is_none() && !context.read_only && !context.busy;
        let create = ui
            .add_enabled_ui(enabled, |ui| {
                filled_button(ui, i18n.t("indexes.builder.create"))
            })
            .inner;
        if create.clicked() {
            action = Some(IndexesAction::Create);
        }
        if text_button(ui, i18n.t("common.cancel")).clicked() {
            action = Some(IndexesAction::CancelBuilder);
        }
        if let Some(problem) = &problem {
            ui.colored_label(ui.visuals().error_fg_color, problem_text(problem, i18n));
        }
    });
    action
}

fn problem_text(problem: &BuilderProblem, i18n: &I18n) -> String {
    match (problem.field(), problem.index_name()) {
        (Some(field), _) => i18n.t_with(problem.message_key(), &[("field", field)]),
        // The buttons that fix it are named by their own labels, so a rename shows here too.
        (None, Some(name)) => i18n.t_with(
            problem.message_key(),
            &[
                ("name", name),
                ("ttl", &i18n.t("indexes.action_ttl")),
                ("hide", &i18n.t("indexes.action_hide")),
                ("unhide", &i18n.t("indexes.action_unhide")),
            ],
        ),
        (None, None) => i18n.t(problem.message_key()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The English locale with the three action labels renamed, as a rename in `en.json` would.
    fn renamed_actions() -> I18n {
        let json = include_str!("../../locales/en.json")
            .replace(
                "\"action_ttl\": \"Change TTL...\"",
                "\"action_ttl\": \"Zeta\"",
            )
            .replace(
                "\"action_hide\": \"Hide from planner\"",
                "\"action_hide\": \"Eta\"",
            )
            .replace(
                "\"action_unhide\": \"Unhide\"",
                "\"action_unhide\": \"Theta\"",
            );
        I18n::from_json(&json).expect("a valid locale")
    }

    #[test]
    fn the_same_keys_message_names_the_actions_by_their_labels() {
        let text = problem_text(
            &BuilderProblem::SameKeysAsExisting("a_1".into()),
            &renamed_actions(),
        );
        for label in ["Zeta", "Eta", "Theta", "a_1"] {
            assert!(text.contains(label), "{label} missing from: {text}");
        }
        assert!(!text.contains('{'), "unfilled placeholder in: {text}");
    }

    #[test]
    fn the_id_message_does_not_offer_the_actions_the_server_refuses() {
        let i18n = renamed_actions();
        let text = problem_text(&BuilderProblem::IdIndexExists, &i18n);
        assert!(text.contains("_id"), "{text}");
        assert!(!text.contains("Zeta") && !text.contains("⟦"), "{text}");
    }
}
