//! The bulk update / delete dialog in the accesskit tree: every control of every step is named,
//! the destructive button is off until the write is confirmed, Cancel owns the first focus,
//! Enter never confirms and a running write cannot be dismissed. Fast, offline, no GPU.
use bson::doc;
use curlybrace_core::bulk::{
    BulkFailureReason, BulkOutcome, PreviewDoc, PreviewGap, UpdatePreview,
};
use curlybrace_core::error::CoreError;
use curlybrace_core::store::Namespace;
use curlybrace_ui::I18n;
use curlybrace_ui::app::BulkAction;
use curlybrace_ui::viewmodel::{
    BulkEntry, BulkFlow, BulkKind, BulkStep, ConnectionBadge, CountState, PreviewState, TreeState,
    ViewMode,
};
use curlybrace_ui::views::bulk_dialog::show_bulk_dialog;
use curlybrace_ui::views::documents::{DocTarget, show_documents};
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const GOOD: &str = r#"{"$set": {"a": 1}}"#;

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn tw(key: &str, params: &[(&str, &str)]) -> String {
    I18n::english().t_with(key, params)
}

struct Frame {
    flow: BulkFlow,
    badge: ConnectionBadge,
    seen: Vec<BulkAction>,
}

fn base(kind: BulkKind) -> BulkFlow {
    BulkFlow::open(
        kind,
        Namespace::new("shop", "orders"),
        doc! { "status": "old" },
        false,
    )
}

fn with_step(flow: BulkFlow, step: BulkStep) -> BulkFlow {
    BulkFlow { step, ..flow }
}

fn update_preview(gap: Option<PreviewGap>, with_after: bool) -> PreviewState {
    let after = with_after.then(|| doc! { "_id": 1, "status": "new" });
    PreviewState::Update(UpdatePreview {
        docs: vec![PreviewDoc {
            before: doc! { "_id": 1, "status": "old" },
            after,
        }],
        gap,
    })
}

fn preview_step(flow: BulkFlow, preview: PreviewState) -> BulkFlow {
    BulkFlow {
        update_text: GOOD.into(),
        count: CountState::Known(5),
        preview,
        ..with_step(flow, BulkStep::Preview)
    }
}

fn confirm_step(flow: BulkFlow) -> BulkFlow {
    BulkFlow {
        update_text: GOOD.into(),
        count: CountState::Known(5),
        preview: update_preview(None, true),
        ..with_step(flow, BulkStep::Confirm)
    }
}

fn open(flow: BulkFlow) -> Harness<'static, Frame> {
    let production = flow.production;
    let frame = Frame {
        flow,
        badge: ConnectionBadge::new(None, production),
        seen: Vec::new(),
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 1100.0))
        .build_state(
            |ctx, f: &mut Frame| {
                let action = show_bulk_dialog(ctx, &f.flow, "local", &f.badge, &I18n::english());
                if let Some(a) = &action {
                    f.flow = applied(f.flow.clone(), a);
                }
                f.seen.extend(action);
            },
            frame,
        );
    harness.run_steps(10);
    harness
}

/// The edits the reducer would make, so typing and ticking can be followed over frames.
fn applied(flow: BulkFlow, action: &BulkAction) -> BulkFlow {
    match action {
        BulkAction::SetUpdateText(s) => flow.with_update_text(s.clone()),
        BulkAction::SetUpsert(on) => flow.with_upsert(*on),
        BulkAction::SetAllDocuments(on) => flow.with_all_documents(*on),
        BulkAction::SetTyped(s) => flow.with_typed(s.clone()),
        _ => flow,
    }
}

fn button<'a>(h: &'a Harness<'static, Frame>, label: &'a str) -> egui_kittest::Node<'a> {
    h.get_by_role_and_label(Role::Button, label)
}

fn has(h: &Harness<'static, Frame>, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

fn disabled(h: &Harness<'static, Frame>, label: &str) -> bool {
    button(h, label).accesskit_node().is_disabled()
}

fn name_of(node: &egui_kittest::Node<'_>) -> Option<String> {
    node.accesskit_node().label().or_else(|| {
        node.accesskit_node()
            .labelled_by()
            .next()
            .and_then(|caption| caption.label())
    })
}

/// Every button, check box and text box on screen has a non-empty name.
fn assert_all_named(h: &Harness<'static, Frame>, what: &str) {
    for role in [
        Role::Button,
        Role::CheckBox,
        Role::TextInput,
        Role::MultilineTextInput,
    ] {
        for node in h.query_all_by_role(role) {
            let name = name_of(&node);
            assert!(
                name.as_deref().is_some_and(|n| !n.trim().is_empty()),
                "{what}: a {role:?} has no name"
            );
        }
    }
}

#[test]
fn define_update_controls_are_named_and_next_waits_for_a_valid_update() {
    let h = open(base(BulkKind::Update));
    h.get_by_label(&t("bulk.title_update"));
    h.get_by_role_and_label(Role::MultilineTextInput, &t("bulk.define.update_field"));
    h.get_by_role_and_label(Role::CheckBox, &t("bulk.define.upsert"));
    h.get_by_label(&t("bulk.filter_scope"));
    assert!(disabled(&h, &t("bulk.next")), "empty update");
    assert_all_named(&h, "define");

    for (text, key) in [
        ("{", "errors.invalid_json"),
        (r#"{"a": 1}"#, "errors.bulk_invalid_update"),
    ] {
        let h = open(BulkFlow {
            update_text: text.into(),
            ..base(BulkKind::Update)
        });
        let label = t("bulk.define.update_field");
        let field = h.get_by_role_and_label(Role::MultilineTextInput, &label);
        assert_eq!(field.accesskit_node().description(), Some(t(key)), "{text}");
        assert!(disabled(&h, &t("bulk.next")), "{text}");
    }
    let ok = open(BulkFlow {
        update_text: GOOD.into(),
        ..base(BulkKind::Update)
    });
    assert!(!disabled(&ok, &t("bulk.next")));
}

#[test]
fn define_delete_has_no_editor_and_names_what_goes() {
    let h = open(base(BulkKind::Delete));
    h.get_by_label(&t("bulk.title_delete"));
    h.get_by_label(&tw(
        "bulk.define.delete_statement",
        &[("collection", "orders")],
    ));
    assert_eq!(h.query_all_by_role(Role::MultilineTextInput).count(), 0);
    assert!(!disabled(&h, &t("bulk.next")));
    assert_all_named(&h, "define delete");
}

/// Cancel owns the first focus of Define, Preview and Confirm. The Running step is the one
/// exception (see `running_step_stop_button_is_labelled_distinctly_and_not_default_focused`).
#[test]
fn cancel_owns_the_first_focus_and_enter_never_advances() {
    let mut h = open(BulkFlow {
        update_text: GOOD.into(),
        ..base(BulkKind::Update)
    });
    assert!(
        button(&h, &t("common.cancel"))
            .accesskit_node()
            .is_focused()
    );
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    let seen = &h.state().seen;
    assert!(
        seen.iter()
            .all(|a| !matches!(a, BulkAction::Next | BulkAction::Run)),
        "{seen:?}"
    );
}

#[test]
fn escape_closes_every_step_before_the_run() {
    for flow in [
        base(BulkKind::Update),
        preview_step(base(BulkKind::Update), update_preview(None, true)),
        confirm_step(base(BulkKind::Update)),
    ] {
        let mut h = open(flow);
        h.key_press(egui::Key::Escape);
        h.run_steps(4);
        assert!(h.state().seen.contains(&BulkAction::Close));
    }
}

#[test]
fn preview_shows_count_before_after_and_the_warning() {
    let h = open(preview_step(
        base(BulkKind::Update),
        update_preview(None, true),
    ));
    h.get_by_label(&tw("bulk.count_known", &[("n", "5")]));
    assert!(has(&h, &t("bulk.cannot_undo")));
    assert!(has(&h, &t("bulk.preview.before")));
    assert!(has(&h, &t("bulk.preview.after")));
    assert!(!disabled(&h, &t("bulk.next")));
    assert!(!disabled(&h, &t("bulk.back")));
    assert_all_named(&h, "preview");
}

#[test]
fn preview_without_an_after_state_says_why_per_gap() {
    for (gap, key) in [
        (PreviewGap::TransactionsUnavailable, "bulk.gap_transactions"),
        (PreviewGap::Conflict, "bulk.gap_conflict"),
    ] {
        let h = open(preview_step(
            base(BulkKind::Update),
            update_preview(Some(gap), false),
        ));
        assert!(has(&h, &t(key)), "{key}");
        assert!(has(&h, &t("bulk.no_after_state")), "{key}");
        assert!(!has(&h, &t("bulk.preview.after")), "{key}");
    }
    let none = open(preview_step(
        base(BulkKind::Update),
        PreviewState::Update(UpdatePreview {
            docs: vec![],
            gap: Some(PreviewGap::NoMatch),
        }),
    ));
    assert!(has(&none, &t("bulk.gap_no_match")));
}

#[test]
fn an_upsert_with_no_matching_document_is_hinted() {
    let flow = BulkFlow {
        upsert: true,
        ..preview_step(
            base(BulkKind::Update),
            PreviewState::Update(UpdatePreview {
                docs: vec![PreviewDoc {
                    before: doc! {},
                    after: Some(doc! { "a": 1 }),
                }],
                gap: None,
            }),
        )
    };
    assert!(has(&open(flow), &t("bulk.preview.upserted_hint")));
}

#[test]
fn preview_waits_for_the_count_and_the_preview() {
    let h = open(BulkFlow {
        count: CountState::Pending,
        preview: PreviewState::Pending,
        ..preview_step(base(BulkKind::Update), PreviewState::Pending)
    });
    assert!(has(&h, &t("bulk.count_pending")));
    assert!(has(&h, &t("bulk.preview_pending")));
    assert!(disabled(&h, &t("bulk.next")));

    let unavailable = open(BulkFlow {
        count: CountState::Unavailable,
        ..preview_step(
            base(BulkKind::Update),
            PreviewState::Failed(CoreError::BulkPreviewUnavailable),
        )
    });
    assert!(has(&unavailable, &t("bulk.count_unavailable")));
    assert!(has(&unavailable, &t("bulk.preview_failed")));
    assert!(!disabled(&unavailable, &t("bulk.next")));
}

#[test]
fn a_preview_that_proved_the_update_invalid_says_so_and_blocks_next_but_not_back() {
    let h = open(preview_step(
        base(BulkKind::Update),
        PreviewState::Failed(CoreError::BulkInvalidUpdate),
    ));
    assert!(has(&h, &t("bulk.preview_update_invalid")));
    assert!(has(&h, &t("errors.bulk_invalid_update")));
    assert!(!has(&h, &t("bulk.preview_failed")));
    assert!(disabled(&h, &t("bulk.next")));
    assert!(!disabled(&h, &t("bulk.back")));
    assert_all_named(&h, "invalid preview");
}

#[test]
fn an_unavailable_preview_shows_its_own_error_text_and_keeps_next() {
    let h = open(preview_step(
        base(BulkKind::Update),
        PreviewState::Failed(CoreError::InvalidJson("secret detail".into())),
    ));
    assert!(has(&h, &t("bulk.preview_failed")));
    assert!(has(&h, &t("errors.invalid_json")));
    assert!(!has(&h, "secret detail"));
    assert!(!has(&h, &t("bulk.preview_update_invalid")));
    assert!(!disabled(&h, &t("bulk.next")));
}

#[test]
fn delete_preview_lists_the_documents_that_would_go() {
    let h = open(BulkFlow {
        count: CountState::Known(2),
        preview: PreviewState::Sample(vec![doc! { "_id": 1 }, doc! { "_id": 2 }]),
        ..with_step(base(BulkKind::Delete), BulkStep::Preview)
    });
    assert!(has(&h, &t("bulk.preview.removed_heading")));
    assert!(has(&h, &t("bulk.cannot_undo")));
    assert_all_named(&h, "delete preview");
}

#[test]
fn run_is_disabled_until_the_collection_name_is_typed() {
    let mut h = open(BulkFlow {
        production: false,
        count: CountState::Known(500),
        ..confirm_step(base(BulkKind::Update))
    });
    let run = t("bulk.run_update");
    assert!(disabled(&h, &run));
    assert!(has(&h, &t("bulk.run_blocked")));
    assert!(has(&h, &t("bulk.cannot_undo")));
    let field = tw("bulk.type_collection_name", &[("name", "orders")]);
    h.get_by_role_and_label(Role::TextInput, &field).click();
    h.get_by_role_and_label(Role::TextInput, &field)
        .type_text("orders");
    h.run_steps(5);
    assert!(!disabled(&h, &run));
    assert!(!has(&h, &t("bulk.run_blocked")));
    button(&h, &run).click();
    h.run_steps(3);
    assert!(h.state().seen.contains(&BulkAction::Run));
    assert_all_named(&h, "confirm");
}

#[test]
fn an_empty_filter_needs_the_tick_and_the_typed_name() {
    let flow = BulkFlow {
        filter: doc! {},
        ..confirm_step(base(BulkKind::Delete))
    };
    let flow = BulkFlow {
        preview: PreviewState::Sample(vec![doc! { "_id": 1 }]),
        ..flow
    };
    let mut h = open(flow);
    let tick = tw(
        "bulk.confirm.all_documents_tick",
        &[("collection", "orders")],
    );
    h.get_by_role_and_label(Role::CheckBox, &tick);
    assert!(has(
        &h,
        &tw(
            "bulk.confirm.all_documents_warning",
            &[("collection", "orders")]
        )
    ));
    assert!(disabled(&h, &t("bulk.run_delete")));
    h.get_by_role_and_label(Role::CheckBox, &tick).click();
    h.run_steps(4);
    assert!(h.state().seen.contains(&BulkAction::SetAllDocuments(true)));
    assert!(disabled(&h, &t("bulk.run_delete")), "still needs the name");
    assert_all_named(&h, "confirm all documents");
}

#[test]
fn the_production_notice_rides_in_the_one_dialog() {
    let prod = open(BulkFlow {
        production: true,
        ..confirm_step(base(BulkKind::Update))
    });
    let notice = tw("bulk.production_notice", &[("connection", "local")]);
    assert!(has(&prod, &notice));
    assert_eq!(prod.query_all_by_label(&t("bulk.run_update")).count(), 1);
    let plain = open(confirm_step(base(BulkKind::Update)));
    assert!(!has(&plain, &notice));
}

#[test]
fn a_running_write_can_only_be_cancelled_never_dismissed() {
    let mut h = open(BulkFlow {
        run: Some(curlybrace_core::bulk::BulkRunId(uuid::Uuid::nil())),
        ..with_step(confirm_step(base(BulkKind::Update)), BulkStep::Running)
    });
    assert!(has(&h, &t("bulk.running")));
    assert!(h.query_by_label(&t("common.close")).is_none());
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert!(h.state().seen.is_empty(), "{:?}", h.state().seen);
    button(&h, &t("bulk.stop")).click();
    h.run_steps(3);
    assert_eq!(h.state().seen, vec![BulkAction::Cancel]);
    assert_all_named(&h, "running");
}

fn running() -> Harness<'static, Frame> {
    open(BulkFlow {
        run: Some(curlybrace_core::bulk::BulkRunId(uuid::Uuid::nil())),
        ..with_step(confirm_step(base(BulkKind::Update)), BulkStep::Running)
    })
}

#[test]
fn running_step_stop_button_is_labelled_distinctly_and_not_default_focused() {
    let mut h = running();
    let stop = t("bulk.stop");
    assert_ne!(stop, t("common.cancel"));
    assert!(
        h.query_by_label(&t("common.cancel")).is_none(),
        "no generic Cancel"
    );
    assert!(!button(&h, &stop).accesskit_node().is_focused());
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    h.key_press(egui::Key::Space);
    h.run_steps(4);
    assert!(h.state().seen.is_empty(), "{:?}", h.state().seen);
    // Still keyboard-reachable: Tab lands on Stop.
    h.key_press(egui::Key::Tab);
    h.run_steps(4);
    assert!(button(&h, &stop).accesskit_node().is_focused());
}

#[test]
fn running_step_explains_what_stop_does() {
    let h = running();
    let note = t("bulk.running_cancel_note");
    h.get_by_label(&note);
    let label = t("bulk.stop");
    let stop = button(&h, &label);
    assert_eq!(stop.accesskit_node().description(), Some(note));
}

#[test]
fn an_unavailable_cancel_is_stated() {
    let h = open(BulkFlow {
        cancel_unavailable: true,
        ..with_step(confirm_step(base(BulkKind::Update)), BulkStep::Running)
    });
    assert!(has(&h, &t("bulk.cancel_unavailable")));
}

fn reported(kind: BulkKind, report: Result<BulkOutcome, CoreError>) -> Harness<'static, Frame> {
    open(BulkFlow {
        report: Some(report),
        ..with_step(confirm_step(base(kind)), BulkStep::Report)
    })
}

#[test]
fn the_report_states_the_counts_of_the_kind() {
    let applied = BulkOutcome::Applied {
        matched: 7,
        modified: 5,
        upserted: vec![],
        deleted: 0,
    };
    let h = reported(BulkKind::Update, Ok(applied));
    assert!(has(&h, &tw("bulk.report.matched", &[("n", "7")])));
    assert!(has(&h, &tw("bulk.report.modified", &[("n", "5")])));
    assert!(!has(&h, &tw("bulk.report.deleted", &[("n", "0")])));

    let deleted = BulkOutcome::Applied {
        matched: 0,
        modified: 0,
        upserted: vec![],
        deleted: 9,
    };
    let h = reported(BulkKind::Delete, Ok(deleted));
    assert!(has(&h, &tw("bulk.report.deleted", &[("n", "9")])));
    assert!(!has(&h, &tw("bulk.report.modified", &[("n", "0")])));
}

#[test]
fn an_unknown_outcome_says_to_check_the_data() {
    for (cancelled, key) in [
        (false, "bulk.report_unknown"),
        (true, "bulk.report_unknown_cancelled"),
    ] {
        let h = reported(BulkKind::Update, Ok(BulkOutcome::Unknown { cancelled }));
        assert!(has(&h, &t(key)), "{key}");
    }
}

#[test]
fn a_rejected_report_names_the_reason_and_never_claims_nothing_was_written() {
    for reason in [
        BulkFailureReason::DuplicateKey,
        BulkFailureReason::ValidationFailed,
        BulkFailureReason::ImmutableField,
        BulkFailureReason::TypeMismatch,
        BulkFailureReason::Other,
    ] {
        let h = reported(BulkKind::Update, Ok(BulkOutcome::Rejected { reason }));
        let text = tw("bulk.report_rejected", &[("reason", &t(reason.i18n_key()))]);
        assert!(has(&h, &text), "{reason:?}");
        assert!(text.contains("may already have been changed"), "{text}");
        assert!(!has(&h, &t("bulk.report_failed")));
    }
}

#[test]
fn an_unconfirmed_report_says_it_was_applied_but_not_confirmed_and_keeps_the_counts() {
    let unconfirmed = |deleted: u64| BulkOutcome::Unconfirmed {
        matched: 7,
        modified: 5,
        upserted: vec![],
        deleted,
    };
    let notice = t("bulk.report_unconfirmed");
    assert!(
        notice.contains("applied") && notice.contains("durab"),
        "{notice}"
    );
    assert!(!notice.contains("rejected"), "{notice}");
    let h = reported(BulkKind::Update, Ok(unconfirmed(0)));
    assert!(has(&h, &notice));
    assert!(has(&h, &tw("bulk.report.matched", &[("n", "7")])));
    assert!(has(&h, &tw("bulk.report.modified", &[("n", "5")])));
    let h = reported(BulkKind::Delete, Ok(unconfirmed(9)));
    assert!(has(&h, &notice));
    assert!(has(&h, &tw("bulk.report.deleted", &[("n", "9")])));
    // It is not the rejected report, which says nothing is known to have been applied.
    assert!(!has(&h, &t("bulk.report_failed")));
}

#[test]
fn a_failed_report_shows_the_translated_error_never_raw_text() {
    let h = reported(
        BulkKind::Update,
        Err(CoreError::InvalidJson("secret detail".into())),
    );
    assert!(has(&h, &t("bulk.report_failed")));
    assert!(has(&h, &t("errors.invalid_json")));
    assert!(!has(&h, "secret detail"));
}

#[test]
fn the_report_closes_with_close_and_escape() {
    let mut h = reported(
        BulkKind::Delete,
        Ok(BulkOutcome::Unknown { cancelled: false }),
    );
    assert_all_named(&h, "report");
    button(&h, &t("common.close")).click();
    h.run_steps(3);
    assert_eq!(h.state().seen, vec![BulkAction::Close]);
    let mut h = reported(
        BulkKind::Delete,
        Ok(BulkOutcome::Unknown { cancelled: true }),
    );
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().seen.contains(&BulkAction::Close));
}

fn toolbar(entry: BulkEntry) -> Harness<'static, TreeState> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 500.0))
        .build_ui_state(
            move |ui, state: &mut TreeState| {
                let target = DocTarget {
                    connection: "Vault",
                    production: false,
                    read_only: entry == BulkEntry::ReadOnly,
                    bulk: entry,
                };
                let page = curlybrace_core::store::Page {
                    docs: vec![doc! { "_id": 1_i32 }],
                    skip: 0,
                    has_more: false,
                };
                show_documents(ui, ViewMode::Tree, &page, state, target, &I18n::english());
            },
            TreeState::default(),
        );
    h.run_steps(4);
    h
}

#[test]
fn entry_buttons_are_disabled_with_a_reason_when_read_only_or_projected() {
    let labels = [t("bulk.update_matching"), t("bulk.delete_matching")];
    for (entry, reason) in [
        (BulkEntry::ReadOnly, t("connection.read_only_reason")),
        (BulkEntry::Projected, t("bulk.projected_reason")),
        (BulkEntry::Busy, t("bulk.busy_reason")),
    ] {
        let h = toolbar(entry);
        for label in &labels {
            let b = h.get_by_role_and_label(Role::Button, label);
            assert!(b.accesskit_node().is_disabled(), "{entry:?} {label}");
            let described = b.accesskit_node().description();
            assert_eq!(
                described.as_deref(),
                Some(reason.as_str()),
                "{entry:?} {label}"
            );
        }
        h.get_by_label(&reason);
    }
    let h = toolbar(BulkEntry::Available);
    for label in &labels {
        let b = h.get_by_role_and_label(Role::Button, label);
        assert!(!b.accesskit_node().is_disabled(), "{label}");
    }
}
