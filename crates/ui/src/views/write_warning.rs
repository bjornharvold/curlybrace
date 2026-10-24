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
        WriteKind::CreateIndex => "write_warning.message_create_index",
        WriteKind::DropIndex => "write_warning.message_drop_index",
        WriteKind::HideIndex => "write_warning.message_hide_index",
        WriteKind::SetIndexTtl => "write_warning.message_set_index_ttl",
    }
}

/// The warning text for `kind`, naming `connection`.
pub fn warning_text(kind: WriteKind, connection: &str, i18n: &I18n) -> String {
    i18n.t_with(message_key(kind), &[("connection", connection)])
}

/// Stateless initial focus for a production dialog: whenever neither button holds it (a fresh
/// dialog, even right after another one vanished unanswered) Cancel takes it, so a stray Enter
/// can never confirm. The user can still Tab to the other button, which then keeps the focus.
pub fn focus_cancel_unless_focused(cancel: &egui::Response, other: &egui::Response) {
    if !cancel.has_focus() && !other.has_focus() {
        cancel.request_focus();
    }
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
            let confirm = ui.button(i18n.t("write_warning.confirm"));
            focus_cancel_unless_focused(&cancel, &confirm);
            if cancel.clicked() {
                answer = Some(WriteWarningAction::Cancel);
            }
            if confirm.clicked() {
                answer = Some(WriteWarningAction::Confirm);
            }
        });
    });
    answer.or_else(|| modal.should_close().then_some(WriteWarningAction::Cancel))
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

    const KINDS: [WriteKind; 8] = [
        WriteKind::Insert,
        WriteKind::Replace,
        WriteKind::Clone,
        WriteKind::Delete,
        WriteKind::CreateIndex,
        WriteKind::DropIndex,
        WriteKind::HideIndex,
        WriteKind::SetIndexTtl,
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
        /// False while no write is parked, as when the reducer dropped it unanswered.
        shown: bool,
        answers: Vec<WriteWarningAction>,
    }

    fn harness(kind: WriteKind) -> Harness<'static, Run> {
        let mut h = Harness::new_state(
            |ctx, run: &mut Run| {
                if !run.shown {
                    return;
                }
                let badge = ConnectionBadge::new(Some(ConnectionColor::Red), true);
                let i18n = I18n::english();
                let shown = show_write_warning(ctx, &run.parked, "prod-db", &badge, &i18n);
                run.answers.extend(shown);
            },
            Run {
                parked: parked(kind),
                shown: true,
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
        assert_eq!(distinct.len(), KINDS.len());
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
    fn a_warning_dropped_unanswered_does_not_stop_the_next_one_focusing_cancel() {
        let cancel = I18n::english().t("common.cancel");
        for frames_between in [0, 2] {
            let mut h = harness(WriteKind::Insert);
            assert!(h.get_by_label(&cancel).accesskit_node().is_focused());
            // The reducer drops the parked write (no answer), then another one is parked.
            h.state_mut().shown = false;
            h.run_steps(frames_between);
            h.state_mut().shown = true;
            h.state_mut().parked = parked(WriteKind::Insert);
            h.run_steps(4);
            assert!(
                h.get_by_label(&cancel).accesskit_node().is_focused(),
                "frames_between={frames_between}"
            );
            assert!(h.state().answers.is_empty());
        }
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
