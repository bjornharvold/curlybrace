//! The small modals of the index writes: the typed confirmation of a drop, the question after a
//! copy was built, and the TTL input. Each draws the state it is given and reports what the
//! person did; the reducer decides what happens. A modal holds still (its buttons are disabled)
//! while a write is in flight, and a write control is disabled on a read-only connection, with
//! the reason in plain text.
use super::connection_badge::show_badge;
use crate::I18n;
use crate::viewmodel::connection_tag::READ_ONLY_REASON_KEY;
use crate::viewmodel::{
    ConnectionBadge, DropConfirm, DropOld, IndexesAction, TtlPrompt, index_write::ttl_problem,
};
use eframe::egui;

const MODAL_WIDTH: f32 = 480.0;
const INPUT_WIDTH: f32 = 260.0;

/// What the modals need besides their own state.
pub struct DialogContext<'a> {
    /// The label of the connection the indexes belong to.
    pub connection: &'a str,
    pub badge: &'a ConnectionBadge,
    /// `database.collection` of the indexes.
    pub collection: &'a str,
    /// A write is in flight: the modal waits for it.
    pub busy: bool,
}

/// The title row with the connection's badge, as the other destructive confirms have it.
fn heading(ui: &mut egui::Ui, title_key: &str, context: &DialogContext<'_>, i18n: &I18n) {
    ui.horizontal(|ui| {
        show_badge(ui, context.badge, i18n);
        ui.heading(i18n.t(title_key));
    });
}

/// Gives `target` the focus the first time the modal keyed by `key` is drawn, and never again, so
/// Tab moves through the modal and wraps around it as usual.
fn focus_once(ui: &egui::Ui, target: &egui::Response, key: (&str, &str)) {
    let id = egui::Id::new(key);
    let done = ui.data_mut(|d| d.get_temp::<bool>(id).unwrap_or(false));
    if !done {
        target.request_focus();
        ui.data_mut(|d| d.insert_temp(id, true));
    }
}

/// Why the main button is off, as a line of text: read-only first, then a write in flight.
fn note(ui: &mut egui::Ui, context: &DialogContext<'_>, i18n: &I18n) {
    if context.badge.read_only {
        ui.label(i18n.t(READ_ONLY_REASON_KEY));
    } else if context.busy {
        ui.label(i18n.t("indexes.builder.busy"));
    }
}

/// Drop is enabled only once the exact name was typed (case, spaces and all). The production
/// notice rides in this dialog like the delete-document confirm does; the production warning
/// still follows at send time, as for every guarded write.
pub fn show_drop_confirm(
    ctx: &egui::Context,
    confirm: &DropConfirm,
    context: &DialogContext<'_>,
    i18n: &I18n,
) -> Option<IndexesAction> {
    let mut action = None;
    let modal = egui::Modal::new(egui::Id::new("index_drop_confirm")).show(ctx, |ui| {
        ui.set_width(MODAL_WIDTH);
        heading(ui, "indexes.drop.title", context, i18n);
        ui.label(i18n.t_with(
            "indexes.drop.message",
            &[
                ("name", &confirm.name),
                ("collection", context.collection),
                ("connection", context.connection),
            ],
        ));
        if context.badge.production {
            ui.label(
                egui::RichText::new(i18n.t("write_warning.production_line"))
                    .strong()
                    .color(ui.visuals().error_fg_color),
            );
        }
        let caption = ui.label(i18n.t_with("indexes.drop.type_name", &[("name", &confirm.name)]));
        let mut typed = confirm.typed.clone();
        let input = ui
            .add_enabled(
                !context.busy,
                egui::TextEdit::singleline(&mut typed).desired_width(INPUT_WIDTH),
            )
            .labelled_by(caption.id);
        if input.changed() {
            action = Some(IndexesAction::DropTyped(typed));
        }
        // The text box takes the focus when the modal opens; a stray Enter cannot drop anything
        // because the button stays off until the name is typed.
        focus_once(ui, &input, ("index_drop_focus", &confirm.name));
        note(ui, context, i18n);
        ui.horizontal(|ui| {
            let cancel = ui.add_enabled(!context.busy, egui::Button::new(i18n.t("common.cancel")));
            let enabled = confirm.confirmed() && !context.busy && !context.badge.read_only;
            let drop = ui.add_enabled(enabled, egui::Button::new(i18n.t("indexes.drop.confirm")));
            if cancel.clicked() {
                action = Some(IndexesAction::DropCancelled);
            }
            if drop.clicked() {
                action = Some(IndexesAction::DropConfirmed);
            }
        });
    });
    let dismissed = modal.should_close() && !context.busy;
    action.or(dismissed.then_some(IndexesAction::DropCancelled))
}

/// After a copy was built: drop the old index or keep both. Keep is first and focused, so a
/// stray Enter drops nothing; Escape keeps both.
pub fn show_drop_old(
    ctx: &egui::Context,
    prompt: &DropOld,
    context: &DialogContext<'_>,
    i18n: &I18n,
) -> Option<IndexesAction> {
    let mut action = None;
    let modal = egui::Modal::new(egui::Id::new("index_drop_old")).show(ctx, |ui| {
        ui.set_width(MODAL_WIDTH);
        heading(ui, "indexes.drop_old.title", context, i18n);
        ui.label(i18n.t_with(
            "indexes.drop_old.message",
            &[("new", &prompt.new), ("old", &prompt.old)],
        ));
        if context.badge.production {
            ui.label(
                egui::RichText::new(i18n.t("write_warning.production_line"))
                    .strong()
                    .color(ui.visuals().error_fg_color),
            );
        }
        note(ui, context, i18n);
        ui.horizontal(|ui| {
            let keep = ui.add_enabled(
                !context.busy,
                egui::Button::new(i18n.t("indexes.drop_old.keep")),
            );
            let enabled = !context.busy && !context.badge.read_only;
            let drop = ui.add_enabled(enabled, egui::Button::new(i18n.t("indexes.drop_old.drop")));
            // Keep owns the first focus, so a stray Enter drops nothing.
            focus_once(ui, &keep, ("index_drop_old_focus", &prompt.old));
            if keep.clicked() {
                action = Some(IndexesAction::KeepOld);
            }
            if drop.clicked() {
                action = Some(IndexesAction::ConfirmDropOld);
            }
        });
    });
    let dismissed = modal.should_close() && !context.busy;
    action.or(dismissed.then_some(IndexesAction::KeepOld))
}

/// The expiry input of "Change TTL...". Apply is enabled for a positive whole number of seconds;
/// otherwise the builder's own message says what is wrong.
pub fn show_ttl_prompt(
    ctx: &egui::Context,
    prompt: &TtlPrompt,
    context: &DialogContext<'_>,
    i18n: &I18n,
) -> Option<IndexesAction> {
    let mut action = None;
    let modal = egui::Modal::new(egui::Id::new("index_ttl_prompt")).show(ctx, |ui| {
        ui.set_width(MODAL_WIDTH);
        heading(ui, "indexes.ttl.title", context, i18n);
        ui.label(i18n.t_with("indexes.ttl.message", &[("name", &prompt.name)]));
        ui.horizontal(|ui| {
            let caption = ui.label(i18n.t("indexes.builder.ttl"));
            let mut text = prompt.text.clone();
            let input = ui
                .add_enabled(
                    !context.busy,
                    egui::TextEdit::singleline(&mut text).desired_width(INPUT_WIDTH),
                )
                .labelled_by(caption.id);
            if input.changed() {
                action = Some(IndexesAction::TtlTyped(text));
            }
            focus_once(ui, &input, ("index_ttl_focus", &prompt.name));
        });
        let seconds = prompt.seconds();
        if let Err(problem) = &seconds {
            ui.colored_label(ui.visuals().error_fg_color, i18n.t(problem.message_key()));
        }
        note(ui, context, i18n);
        ui.horizontal(|ui| {
            let ready = seconds.as_ref().ok().filter(|s| ttl_problem(**s).is_none());
            let enabled = ready.is_some() && !context.busy && !context.badge.read_only;
            let apply = ui.add_enabled(enabled, egui::Button::new(i18n.t("indexes.ttl.apply")));
            if let (true, Some(seconds)) = (apply.clicked(), ready) {
                action = Some(IndexesAction::SetTtl(prompt.name.clone(), *seconds));
            }
            let cancel = ui.add_enabled(!context.busy, egui::Button::new(i18n.t("common.cancel")));
            if cancel.clicked() {
                action = Some(IndexesAction::TtlCancelled);
            }
        });
    });
    let dismissed = modal.should_close() && !context.busy;
    action.or(dismissed.then_some(IndexesAction::TtlCancelled))
}
