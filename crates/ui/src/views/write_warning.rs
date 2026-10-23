//! The production write warning: a modal shown while a write to a production connection is
//! parked. Cancel comes first and owns the initial focus, so Enter can never confirm; Escape and
//! a click outside cancel too (fail closed).
use super::connection_badge::show_badge;
use crate::I18n;
use crate::app::write_guard::{ParkedWrite, WriteKind, WriteWarningAction};
use crate::viewmodel::ConnectionBadge;
use eframe::egui;

/// i18n key of the message worded for `kind`.
pub fn message_key(kind: WriteKind) -> &'static str {
    match kind {
        WriteKind::Insert => "write_warning.message_insert",
        WriteKind::Replace => "write_warning.message_replace",
        WriteKind::Clone => "write_warning.message_clone",
        WriteKind::Delete => "write_warning.message_delete",
    }
}

/// The warning text for `kind`, naming `connection`.
pub fn warning_text(kind: WriteKind, connection: &str, i18n: &I18n) -> String {
    i18n.t_with(message_key(kind), &[("connection", connection)])
}

/// Draws the warning for `parked`; returns the user's answer, if any this frame.
pub fn show_write_warning(
    ctx: &egui::Context,
    parked: &ParkedWrite,
    connection: &str,
    badge: &ConnectionBadge,
    i18n: &I18n,
) -> Option<WriteWarningAction> {
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("write_warning")).show(ctx, |ui| {
        ui.horizontal(|ui| {
            show_badge(ui, badge, i18n);
            ui.heading(i18n.t("write_warning.title"));
        });
        ui.label(warning_text(parked.kind, connection, i18n));
        ui.horizontal(|ui| {
            // Cancel is first and focused, so a stray Enter can never confirm.
            let cancel = ui.button(i18n.t("common.cancel"));
            focus_once(ui, &cancel);
            if cancel.clicked() {
                answer = Some(WriteWarningAction::Cancel);
            }
            if ui.button(i18n.t("write_warning.confirm")).clicked() {
                answer = Some(WriteWarningAction::Confirm);
            }
        });
    });
    let answer = answer.or_else(|| modal.should_close().then_some(WriteWarningAction::Cancel));
    if answer.is_some() {
        // The next warning is a new question and gets its own initial focus.
        ctx.data_mut(|d| d.remove::<bool>(focus_id()));
    }
    answer
}

fn focus_id() -> egui::Id {
    egui::Id::new("write_warning_focus")
}

/// Gives `response` keyboard focus the first frame the warning is drawn.
fn focus_once(ui: &egui::Ui, response: &egui::Response) {
    let seen = ui.data_mut(|d| d.get_temp::<bool>(focus_id()).is_some());
    if !seen {
        response.request_focus();
        ui.data_mut(|d| d.insert_temp(focus_id(), true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::Pending;
    use bson::Bson;
    use curlybrace_core::connection::ConnectionColor;
    use curlybrace_core::runtime::Command;
    use curlybrace_core::test_support::mock_objects::mock_namespace;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
    use uuid::Uuid;

    const KINDS: [WriteKind; 4] = [
        WriteKind::Insert,
        WriteKind::Replace,
        WriteKind::Clone,
        WriteKind::Delete,
    ];

    fn parked(kind: WriteKind) -> ParkedWrite {
        let ns = mock_namespace();
        ParkedWrite {
            command: Command::Delete {
                session: Uuid::nil(),
                ns: ns.clone(),
                id: Bson::Int32(1),
            },
            pending: Pending { ns, editor: None },
            kind,
            epoch: 1,
        }
    }

    struct Run {
        parked: ParkedWrite,
        answers: Vec<WriteWarningAction>,
    }

    fn harness(kind: WriteKind) -> Harness<'static, Run> {
        let mut h = Harness::new_state(
            |ctx, run: &mut Run| {
                let badge = ConnectionBadge::new(Some(ConnectionColor::Red), true);
                let i18n = I18n::english();
                let shown = show_write_warning(ctx, &run.parked, "prod-db", &badge, &i18n);
                run.answers.extend(shown);
            },
            Run {
                parked: parked(kind),
                answers: Vec::new(),
            },
        );
        h.run_steps(4);
        h
    }

    #[test]
    fn warning_text_names_the_connection_and_the_action() {
        let i18n = I18n::english();
        let texts: Vec<String> = KINDS
            .into_iter()
            .map(|k| warning_text(k, "prod-db", &i18n))
            .collect();
        for text in &texts {
            assert!(text.contains("prod-db"), "{text}");
            assert!(!text.starts_with('⟦'), "{text}");
        }
        let distinct: std::collections::BTreeSet<_> = texts.iter().collect();
        assert_eq!(distinct.len(), 4);
        // And the modal really draws it.
        let h = harness(WriteKind::Delete);
        h.get_by_label(&warning_text(WriteKind::Delete, "prod-db", &i18n));
    }

    #[test]
    fn cancel_button_has_initial_focus_and_escape_cancels() {
        let i18n = I18n::english();
        let h = harness(WriteKind::Insert);
        let (cancel, confirm) = (i18n.t("common.cancel"), i18n.t("write_warning.confirm"));
        assert!(h.get_by_label(&cancel).accesskit_node().is_focused());
        assert!(!h.get_by_label(&confirm).accesskit_node().is_focused());
        let mut h = harness(WriteKind::Insert);
        h.key_press(egui::Key::Escape);
        h.run_steps(4);
        assert_eq!(h.state().answers, vec![WriteWarningAction::Cancel]);
    }

    #[test]
    fn enter_cancels_because_cancel_is_focused() {
        let mut h = harness(WriteKind::Insert);
        h.key_press(egui::Key::Enter);
        h.run_steps(4);
        assert_eq!(h.state().answers, vec![WriteWarningAction::Cancel]);
    }

    #[test]
    fn confirm_button_confirms() {
        let mut h = harness(WriteKind::Replace);
        h.get_by_label(&I18n::english().t("write_warning.confirm"))
            .click();
        h.run_steps(4);
        assert_eq!(h.state().answers, vec![WriteWarningAction::Confirm]);
    }

    #[test]
    fn all_new_keys_exist_in_en_json() {
        let i18n = I18n::english();
        let keys = [
            "write_warning.title",
            "write_warning.message_insert",
            "write_warning.message_replace",
            "write_warning.message_clone",
            "write_warning.message_delete",
            "write_warning.confirm",
            "write_warning.production_line",
        ];
        for key in keys {
            assert!(!i18n.t(key).starts_with('⟦'), "{key}");
        }
    }
}
