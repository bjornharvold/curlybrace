//! The paste dialog: shown for every paste, before anything is written. It names the source and
//! the target, lets the person choose the policy, the target names and the options, shows the
//! counts as they arrive, and asks for the typed word when the copy overwrites or the target is a
//! production connection. It only reports inputs; whether Copy may fire is decided by the same
//! `CopyDialog::can_confirm` the reducer repeats.
use super::connection_badge::show_badge;
use super::copy_progress::show_progress;
use super::write_warning::focus_cancel_unless_focused;
use crate::I18n;
use crate::app::copy::CopyInput;
use crate::app::copy_state::{CopyDialog, CopyPhase, CountRow};
use crate::app::state::AppState;
use crate::viewmodel::ConnectionBadge;
use curlybrace_core::copy::policy::ConflictPolicy;
use curlybrace_core::copy::protocol::CopyScope;
use eframe::egui;

const MODAL_WIDTH: f32 = 620.0;
const COUNTS_MAX_HEIGHT: f32 = 200.0;
const POLICIES: [(ConflictPolicy, &str); 3] = [
    (ConflictPolicy::Skip, "copy.dialog.policy.skip"),
    (ConflictPolicy::Overwrite, "copy.dialog.policy.overwrite"),
    (ConflictPolicy::Upsert, "copy.dialog.policy.upsert"),
];

/// Who the dialog writes to, as the person knows them.
struct Context {
    source: String,
    target: String,
    source_db: String,
    target_db: String,
    badge: ConnectionBadge,
    production: bool,
}

impl Context {
    fn of(state: &AppState, dialog: &CopyDialog, i18n: &I18n) -> Self {
        let label = |id| {
            state
                .open_label(id, i18n)
                .unwrap_or_else(|| i18n.t("doc.connection_unknown"))
        };
        let request = &dialog.request;
        let (source_db, target_db) = match &request.scope {
            CopyScope::Database { source_db, .. } => (
                source_db.clone(),
                dialog
                    .names
                    .first()
                    .map(|(_, t)| t.clone())
                    .unwrap_or_default(),
            ),
            CopyScope::Collections(items) => items
                .first()
                .map(|i| (i.source.db.clone(), i.target.db.clone()))
                .unwrap_or_default(),
        };
        Self {
            source: label(request.source),
            target: label(request.target),
            source_db,
            target_db,
            badge: state.session_badge(request.target),
            production: state.session_production(request.target),
        }
    }
}

/// The paste dialog, or the progress and report of the copy it started; nothing without one.
pub fn show_copy_modal(ctx: &egui::Context, state: &AppState, i18n: &I18n) -> Option<CopyInput> {
    let dialog = state.copy.as_ref()?;
    match &dialog.phase {
        CopyPhase::Running(run) => show_progress(ctx, run, i18n),
        CopyPhase::Editing | CopyPhase::Starting => {
            show_dialog(ctx, dialog, &Context::of(state, dialog, i18n), i18n)
        }
    }
}

fn show_dialog(
    ctx: &egui::Context,
    dialog: &CopyDialog,
    context: &Context,
    i18n: &I18n,
) -> Option<CopyInput> {
    let modal = egui::Modal::new(egui::Id::new("copy_dialog"));
    let response = modal.show(ctx, |ui| show_body(ui, dialog, context, i18n));
    let closed = response.should_close();
    response.inner.or(closed.then_some(CopyInput::Cancel))
}

fn show_body(
    ui: &mut egui::Ui,
    dialog: &CopyDialog,
    context: &Context,
    i18n: &I18n,
) -> Option<CopyInput> {
    ui.set_width(MODAL_WIDTH);
    show_header(ui, context, i18n);
    let choosing = matches!(dialog.phase, CopyPhase::Editing);
    let mut input = None;
    let mut typing = false;
    ui.add_enabled_ui(choosing, |ui| {
        input = show_policy(ui, dialog, i18n);
        let (renamed, editing) = show_names(ui, dialog, i18n);
        typing = editing;
        input = input.take().or(renamed);
        input = input.take().or(show_options(ui, dialog, i18n));
    });
    show_counts(ui, dialog, i18n);
    let production = context.production;
    let (typed, typing_word) = show_confirmation(ui, dialog, production, choosing, i18n);
    let buttons = show_buttons(ui, dialog, production, typing || typing_word, i18n);
    input.or(typed).or(buttons)
}

fn show_header(ui: &mut egui::Ui, context: &Context, i18n: &I18n) {
    ui.horizontal(|ui| {
        show_badge(ui, &context.badge, i18n);
        ui.heading(i18n.t("copy.dialog.title"));
    });
    let line = |key: &str, connection: &str, database: &str| {
        i18n.t_with(key, &[("connection", connection), ("database", database)])
    };
    ui.label(line(
        "copy.dialog.from",
        &context.source,
        &context.source_db,
    ));
    ui.label(line("copy.dialog.to", &context.target, &context.target_db));
    if context.production {
        let notice = i18n.t_with(
            "copy.dialog.production_line",
            &[("connection", &context.target)],
        );
        let color = ui.visuals().error_fg_color;
        ui.label(egui::RichText::new(notice).strong().color(color));
    }
}

fn show_policy(ui: &mut egui::Ui, dialog: &CopyDialog, i18n: &I18n) -> Option<CopyInput> {
    ui.separator();
    ui.strong(i18n.t("copy.dialog.policy.title"));
    let mut chosen = None;
    for (policy, key) in POLICIES {
        let selected = dialog.policy == policy;
        if ui.radio(selected, i18n.t(key)).clicked() && !selected {
            chosen = Some(CopyInput::SetPolicy(policy));
        }
    }
    chosen
}

/// One text box per target name. Also reports whether one holds the keyboard focus.
fn show_names(ui: &mut egui::Ui, dialog: &CopyDialog, i18n: &I18n) -> (Option<CopyInput>, bool) {
    ui.separator();
    ui.strong(i18n.t("copy.dialog.names_title"));
    let mut changed = None;
    let mut editing = false;
    for (index, (source, target)) in dialog.names.iter().enumerate() {
        ui.push_id(index, |ui| {
            let caption = ui.label(i18n.t_with("copy.dialog.name_label", &[("source", source)]));
            let mut text = target.clone();
            let edit = egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY);
            let response = ui.add(edit).labelled_by(caption.id);
            editing |= response.has_focus();
            if response.changed() {
                changed = Some(CopyInput::SetName { index, name: text });
            }
            if let Some(issue) = dialog.name_issue(index) {
                ui.colored_label(ui.visuals().error_fg_color, i18n.t(issue.i18n_key()));
            }
        });
    }
    (changed, editing)
}

fn show_options(ui: &mut egui::Ui, dialog: &CopyDialog, i18n: &I18n) -> Option<CopyInput> {
    ui.separator();
    let mut checked = dialog.with_indexes_and_options;
    let toggle = ui.checkbox(&mut checked, i18n.t("copy.dialog.indexes_and_options"));
    toggle
        .changed()
        .then_some(CopyInput::ToggleIndexesAndOptions)
}

fn show_counts(ui: &mut egui::Ui, dialog: &CopyDialog, i18n: &I18n) {
    ui.separator();
    ui.strong(i18n.t("copy.dialog.counts.title"));
    if dialog.counting {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(i18n.t("copy.dialog.counting_note"));
        });
    }
    if let Some(error) = &dialog.preview_error {
        let reason = i18n.error(error);
        let text = i18n.t_with("copy.dialog.counts.failed", &[("reason", &reason)]);
        ui.colored_label(ui.visuals().warn_fg_color, text);
    }
    egui::ScrollArea::vertical()
        .id_salt("copy_counts")
        .max_height(COUNTS_MAX_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (index, item) in dialog.items.iter().enumerate() {
                ui.label(count_text(dialog, index, &item.source.collection, i18n));
            }
        });
    ui.weak(i18n.t("copy.dialog.no_snapshot"));
}

fn count_text(dialog: &CopyDialog, index: usize, collection: &str, i18n: &I18n) -> String {
    let named = |key: &str, extra: &[(&str, &str)]| {
        let mut params = vec![("collection", collection)];
        params.extend_from_slice(extra);
        i18n.t_with(key, &params)
    };
    match dialog.count_row(index) {
        CountRow::Waiting => named("copy.dialog.counts.waiting", &[]),
        CountRow::Unavailable => named("copy.dialog.counts.unavailable", &[]),
        CountRow::Counting(scanned) => named(
            "copy.dialog.counts.counting",
            &[("scanned", &scanned.to_string())],
        ),
        CountRow::Counted(c) => named(
            "copy.dialog.counts.counted",
            &[
                ("new", &c.new.to_string()),
                ("changed", &c.changed.to_string()),
                ("unchanged", &c.unchanged.to_string()),
            ],
        ),
    }
}

/// The typed confirmation, when Overwrite or a production target asks for one: the prompt, the
/// text to type and a full-width box. Also reports whether the box holds the keyboard focus.
fn show_confirmation(
    ui: &mut egui::Ui,
    dialog: &CopyDialog,
    production: bool,
    enabled: bool,
    i18n: &I18n,
) -> (Option<CopyInput>, bool) {
    if !dialog.needs_typed_confirmation(production) {
        return (None, false);
    }
    ui.separator();
    if dialog.policy == ConflictPolicy::Overwrite {
        ui.label(i18n.t("copy.dialog.overwrite_note"));
    }
    // The prompt carries the required text, so the box's accessible name states it too.
    let prompt = i18n.t_with(
        "copy.dialog.confirm_prompt",
        &[("text", &dialog.required_text())],
    );
    let caption = ui.label(egui::RichText::new(prompt).strong());
    let mut text = dialog.typed.clone();
    let edit = egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY);
    let response = ui.add_enabled(enabled, edit).labelled_by(caption.id);
    let input = response
        .changed()
        .then_some(CopyInput::TypeConfirmation(text));
    (input, response.has_focus())
}

/// Cancel comes first. When the copy overwrites or the target is production, Cancel also holds
/// the initial focus, so Enter can never confirm; it leaves the focus alone while the person is
/// typing into any box.
fn show_buttons(
    ui: &mut egui::Ui,
    dialog: &CopyDialog,
    production: bool,
    typing: bool,
    i18n: &I18n,
) -> Option<CopyInput> {
    ui.separator();
    let allowed = dialog.can_confirm(production);
    if !allowed && matches!(dialog.phase, CopyPhase::Editing) {
        let key = match dialog.names_valid() {
            true => "copy.dialog.blocked_typed",
            false => "copy.dialog.blocked_names",
        };
        ui.colored_label(ui.visuals().warn_fg_color, i18n.t(key));
    }
    ui.horizontal(|ui| {
        let cancel = ui.button(i18n.t("common.cancel"));
        let copy = ui.add_enabled(allowed, egui::Button::new(i18n.t("copy.dialog.copy")));
        if dialog.needs_typed_confirmation(production) && !typing {
            focus_cancel_unless_focused(&cancel, &copy);
        }
        if matches!(dialog.phase, CopyPhase::Starting) {
            ui.spinner();
            ui.label(i18n.t("copy.progress.cancelling"));
        }
        copy.clicked()
            .then_some(CopyInput::Confirm)
            .or(cancel.clicked().then_some(CopyInput::Cancel))
    })
    .inner
}
