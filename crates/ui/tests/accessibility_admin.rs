//! The administration dialogs (drop, rename, duplicate, create view) in the accesskit tree: every
//! control is named and reachable, a drop needs the exact name typed, the send button is off for
//! a read-only connection (with the reason in text) and while a write is in flight, and every
//! string comes from an i18n key. Fast, offline, no GPU.
use curlybrace_ui::I18n;
use curlybrace_ui::viewmodel::{
    AdminAction, AdminDialog, AdminKind, AdminRequest, ConnectionBadge,
};
use curlybrace_ui::views::admin_dialogs::{AdminContext, show_admin_dialog, text_key};
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn t(key: &str) -> String {
    I18n::english().t(key)
}

struct Frame {
    dialog: AdminDialog,
    badge: ConnectionBadge,
    busy: bool,
    seen: Vec<AdminAction>,
}

fn dialog(kind: AdminKind, collection: Option<&str>) -> AdminDialog {
    AdminDialog::open(AdminRequest {
        kind,
        db: "shop".into(),
        collection: collection.map(str::to_owned),
    })
}

fn open(dialog: AdminDialog, badge: ConnectionBadge, busy: bool) -> Harness<'static, Frame> {
    let frame = Frame {
        dialog,
        badge,
        busy,
        seen: Vec::new(),
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 800.0))
        .build_state(
            |ctx, frame: &mut Frame| {
                let context = AdminContext {
                    connection: "local",
                    badge: &frame.badge,
                    busy: frame.busy,
                };
                let action = show_admin_dialog(ctx, &frame.dialog, &context, &I18n::english());
                // Edits are applied the way the reducer would, to follow them over frames.
                if let Some(action) = &action {
                    let d = frame.dialog.clone();
                    frame.dialog = match action.clone() {
                        AdminAction::Typed(s) => d.with_typed(s),
                        AdminAction::NameEdited(s) => d.with_name(s),
                        AdminAction::SourceEdited(s) => d.with_source(s),
                        AdminAction::PipelineEdited(s) => d.with_pipeline(s),
                        AdminAction::DropTargetToggled(on) => d.with_drop_target(on),
                        _ => d,
                    };
                }
                frame.seen.extend(action);
            },
            frame,
        );
    harness.run_steps(10);
    harness
}

fn disabled(harness: &Harness<'static, Frame>, label: &str) -> bool {
    harness
        .get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_disabled()
}

fn calm() -> ConnectionBadge {
    ConnectionBadge::default()
}

const KINDS: [AdminKind; 5] = [
    AdminKind::DropCollection,
    AdminKind::DropDatabase,
    AdminKind::Rename,
    AdminKind::Duplicate,
    AdminKind::CreateView,
];

#[test]
fn every_dialog_has_a_title_and_named_cancel_and_send_buttons() {
    for kind in KINDS {
        let harness = open(dialog(kind, Some("orders")), calm(), false);
        // A drop's title and its send button read the same, so only presence is asserted.
        assert!(
            harness
                .query_all_by_label(&t(&text_key("title", kind)))
                .next()
                .is_some(),
            "{kind:?} has a title"
        );
        harness.get_by_role_and_label(Role::Button, &t("common.cancel"));
        harness.get_by_role_and_label(Role::Button, &t(&text_key("confirm", kind)));
    }
}

#[test]
fn a_drop_needs_the_exact_name_typed_and_reports_the_confirmation_once() {
    let drop = t("admin.confirm.drop_collection");
    let caption = I18n::english().t_with("admin.typed.accessible", &[("name", "orders")]);
    let mut harness = open(
        dialog(AdminKind::DropCollection, Some("orders")),
        calm(),
        false,
    );
    assert!(disabled(&harness, &drop), "off until the name is typed");
    harness
        .get_by_role_and_label(Role::TextInput, &caption)
        .type_text("Orders");
    harness.run_steps(4);
    assert_eq!(
        harness.state().seen.last(),
        Some(&AdminAction::Typed("Orders".into()))
    );
    assert!(
        disabled(&harness, &drop),
        "a different case does not confirm"
    );

    let exact = dialog(AdminKind::DropCollection, Some("orders")).with_typed("orders".into());
    let mut harness = open(exact, calm(), false);
    assert!(!disabled(&harness, &drop));
    harness.get_by_role_and_label(Role::Button, &drop).click();
    harness.run_steps(3);
    assert_eq!(harness.state().seen, vec![AdminAction::Confirmed]);
}

#[test]
fn the_database_drop_names_the_database_to_type() {
    let caption = I18n::english().t_with("admin.typed.accessible", &[("name", "shop")]);
    let harness = open(dialog(AdminKind::DropDatabase, None), calm(), false);
    harness.get_by_role_and_label(Role::TextInput, &caption);
    assert!(disabled(&harness, &t("admin.confirm.drop_database")));
}

#[test]
fn rename_and_duplicate_offer_their_name_field_and_the_replace_choice() {
    let rename = open(dialog(AdminKind::Rename, Some("orders")), calm(), false);
    rename.get_by_role_and_label(Role::TextInput, &t("admin.field.new_name"));
    rename.get_by_label(&t("admin.field.replace_existing"));
    // The name is unchanged, so the send button is off and the reason is in text.
    assert!(disabled(&rename, &t("admin.confirm.rename")));
    rename.get_by_label(&t("admin.problem.same_name"));

    let copy = open(dialog(AdminKind::Duplicate, Some("orders")), calm(), false);
    copy.get_by_role_and_label(Role::TextInput, &t("admin.field.copy_name"));
    assert!(!disabled(&copy, &t("admin.confirm.duplicate")));
}

#[test]
fn a_view_dialog_names_its_three_inputs_and_explains_a_bad_pipeline() {
    let view = dialog(AdminKind::CreateView, Some("orders"))
        .with_name("recent".into())
        .with_pipeline("{".into());
    let harness = open(view, calm(), false);
    for key in ["admin.field.view_name", "admin.field.view_source"] {
        harness.get_by_role_and_label(Role::TextInput, &t(key));
    }
    // The pipeline is a box of several lines.
    harness.get_by_role_and_label(Role::MultilineTextInput, &t("admin.field.view_pipeline"));
    assert!(disabled(&harness, &t("admin.confirm.create_view")));
    harness.get_by_label(&t("admin.problem.pipeline"));
}

#[test]
fn a_refusal_from_the_core_is_shown_in_words() {
    let refused = dialog(AdminKind::Duplicate, Some("orders"))
        .with_error(Some(curlybrace_core::error::CoreError::NamespaceExists));
    let harness = open(refused, calm(), false);
    harness
        .get_by_label(&I18n::english().error(&curlybrace_core::error::CoreError::NamespaceExists));
}

#[test]
fn a_read_only_connection_cannot_send_and_says_why() {
    let ready = dialog(AdminKind::Duplicate, Some("orders"));
    let harness = open(ready, calm().with_read_only(true), false);
    assert!(disabled(&harness, &t("admin.confirm.duplicate")));
    harness.get_by_label(&t("connection.read_only_reason"));
}

#[test]
fn the_production_notice_shows_and_a_busy_dialog_holds_still() {
    let ready = dialog(AdminKind::Duplicate, Some("orders"));
    let production = open(ready.clone(), ConnectionBadge::new(None, true), false);
    production.get_by_label(&t("write_warning.production_line"));
    let calm_harness = open(ready.clone(), calm(), false);
    assert_eq!(
        calm_harness
            .query_all_by_label(&t("write_warning.production_line"))
            .count(),
        0
    );
    let busy = open(ready, calm(), true);
    assert!(disabled(&busy, &t("admin.confirm.duplicate")));
    assert!(disabled(&busy, &t("common.cancel")));
}
