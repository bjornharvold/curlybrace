//! The index builder modal: ordered key rows, the options, the exact command that will be sent,
//! and Create / Cancel. Draws the state it is given and reports what the person did; the reducer
//! decides what happens.
use crate::I18n;
use crate::viewmodel::connection_tag::READ_ONLY_REASON_KEY;
use crate::viewmodel::index_builder::{BUILDER_KINDS, kind_label_key};
use crate::viewmodel::{
    BuildWarning, BuilderEdit, BuilderProblem, IndexBuilderState, IndexesAction,
};
use curlybrace_core::indexes::KeyKind;
use eframe::egui::{self, WidgetInfo, WidgetType};

const MODAL_WIDTH: f32 = 640.0;
const FIELD_WIDTH: f32 = 220.0;
const NAME_WIDTH: f32 = 300.0;
const TTL_WIDTH: f32 = 100.0;
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
    let modal = egui::Modal::new(egui::Id::new("index_builder")).show(ctx, |ui| {
        ui.set_width(MODAL_WIDTH);
        let namespace = format!("{}.{}", builder.ns.db, builder.ns.collection);
        ui.heading(i18n.t("indexes.builder.title"));
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
                action = body(ui, builder, i18n).map(IndexesAction::Builder);
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
    ui.strong(i18n.t("indexes.builder.keys_heading"));
    edit = key_rows(ui, builder, i18n).or(edit);
    if ui.button(i18n.t("indexes.builder.add_key")).clicked() {
        edit = Some(BuilderEdit::AddKey);
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
    for (i, key) in builder.keys.iter().enumerate() {
        let n = (i + 1).to_string();
        let numbered = [("n", n.as_str())];
        ui.horizontal(|ui| {
            let caption = ui.label(i18n.t_with("indexes.builder.key_field_n", &numbered));
            let mut text = key.field.clone();
            let field = ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(FIELD_WIDTH))
                .labelled_by(caption.id);
            if field.changed() {
                edit = Some(BuilderEdit::SetField(i, text));
            }
            if let Some(kind) = kind_combo(ui, i, &key.kind, &numbered, i18n) {
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
    }
    edit
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
    let response = ui.add_enabled(enabled, egui::Button::new(i18n.t(button.text)));
    let name = i18n.t_with(button.name, params);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, name.clone()));
    response.clicked()
}

/// The key type picker; the kinds the builder does not offer show up only as the current value of
/// a key that already has one.
fn kind_combo(
    ui: &mut egui::Ui,
    index: usize,
    current: &KeyKind,
    params: &[(&str, &str)],
    i18n: &I18n,
) -> Option<KeyKind> {
    let mut chosen = None;
    let shown = i18n.t(kind_label_key(current));
    let combo = egui::ComboBox::from_id_salt(("index_key_kind", index))
        .selected_text(shown.clone())
        .show_ui(ui, |ui| {
            for kind in BUILDER_KINDS {
                let label = i18n.t(kind_label_key(&kind));
                if ui.selectable_label(*current == kind, label).clicked() {
                    chosen = Some(kind);
                }
            }
        });
    let name = i18n.t_with("indexes.builder.key_type_n", params);
    combo.response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::ComboBox, true, name.clone());
        info.current_text_value = Some(shown.clone());
        info
    });
    chosen
}

fn options(ui: &mut egui::Ui, builder: &IndexBuilderState, i18n: &I18n) -> Option<BuilderEdit> {
    let mut edit = None;
    ui.horizontal(|ui| {
        let caption = ui.label(i18n.t("indexes.builder.name"));
        let mut name = builder.name.clone();
        let response = ui
            .add(egui::TextEdit::singleline(&mut name).desired_width(NAME_WIDTH))
            .labelled_by(caption.id);
        if response.changed() {
            edit = Some(BuilderEdit::SetName(name));
        }
    });
    ui.horizontal(|ui| {
        let caption = ui.label(i18n.t("indexes.builder.ttl"));
        let mut text = builder
            .options
            .expire_after_seconds
            .map(|s| s.to_string())
            .unwrap_or_default();
        let response = ui
            .add(egui::TextEdit::singleline(&mut text).desired_width(TTL_WIDTH))
            .labelled_by(caption.id);
        // Text that is not a whole number is not an edit: the box shows the value again.
        if let (true, Some(ttl)) = (response.changed(), parse_seconds(&text)) {
            edit = Some(BuilderEdit::SetTtl(ttl));
        }
    });
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
    let collation = json_box(
        ui,
        "indexes.builder.collation",
        &builder.collation_text,
        i18n,
    );
    let partial = json_box(
        ui,
        "indexes.builder.partial_filter",
        &builder.partial_filter_text,
        i18n,
    );
    collation
        .map(BuilderEdit::SetCollationText)
        .or(partial.map(BuilderEdit::SetPartialFilterText))
        .or(edit)
}

/// `Some(None)` for blank text, `Some(Some(n))` for a whole number, `None` for anything else.
fn parse_seconds(text: &str) -> Option<Option<i64>> {
    match text.trim().is_empty() {
        true => Some(None),
        false => text.trim().parse().ok().map(Some),
    }
}

/// A captioned multi-line text box; returns the new text when it changed.
fn json_box(ui: &mut egui::Ui, caption_key: &str, current: &str, i18n: &I18n) -> Option<String> {
    let caption = ui.label(i18n.t(caption_key));
    let mut text = current.to_string();
    let response = ui
        .add(
            egui::TextEdit::multiline(&mut text)
                .font(egui::TextStyle::Monospace)
                // Tab moves on to the next control instead of typing a tab character.
                .lock_focus(false)
                .desired_rows(JSON_ROWS)
                .desired_width(f32::INFINITY),
        )
        .labelled_by(caption.id);
    response.changed().then_some(text)
}

/// The command exactly as the store will receive it, read-only but selectable.
fn preview(ui: &mut egui::Ui, builder: &IndexBuilderState, i18n: &I18n) {
    let caption = ui.strong(i18n.t("indexes.builder.preview_heading"));
    let json = builder.preview_json();
    let mut shown: &str = &json;
    ui.add(
        egui::TextEdit::multiline(&mut shown)
            .font(egui::TextStyle::Monospace)
            .lock_focus(false)
            .desired_rows(
                json.lines()
                    .count()
                    .clamp(*PREVIEW_ROWS.start(), *PREVIEW_ROWS.end()),
            )
            .desired_width(f32::INFINITY),
    )
    .labelled_by(caption.id);
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
        ui.colored_label(ui.visuals().warn_fg_color, text);
    }
    let problem = builder.problem();
    if context.read_only {
        ui.label(i18n.t(READ_ONLY_REASON_KEY));
    }
    if context.busy {
        ui.label(i18n.t("indexes.builder.busy"));
    }
    let mut action = None;
    ui.horizontal(|ui| {
        let enabled = problem.is_none() && !context.read_only && !context.busy;
        let create = ui.add_enabled(enabled, egui::Button::new(i18n.t("indexes.builder.create")));
        if create.clicked() {
            action = Some(IndexesAction::Create);
        }
        if ui.button(i18n.t("common.cancel")).clicked() {
            action = Some(IndexesAction::CancelBuilder);
        }
        if let Some(problem) = &problem {
            ui.colored_label(ui.visuals().error_fg_color, problem_text(problem, i18n));
        }
    });
    action
}

fn problem_text(problem: &BuilderProblem, i18n: &I18n) -> String {
    match problem.field() {
        Some(field) => i18n.t_with(problem.message_key(), &[("field", field)]),
        None => i18n.t(problem.message_key()),
    }
}
