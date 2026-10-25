//! The paste dialog, its progress and its report: pure reducer tests over the copy events.
use super::copy_state::{CopyPhase, NameIssue, RunOutcome};
use super::reducer::{Input, update};
use super::session::SessionView;
use super::state::{AppState, ConnStatus};
use crate::viewmodel::{CopyClipboard, PasteTarget, SidebarAction, SidebarState};
use curlybrace_core::copy::engine::{CopyOutcome, CopyProgress, CopyReport};
use curlybrace_core::copy::policy::ConflictPolicy;
use curlybrace_core::copy::preview::PreviewCounts;
use curlybrace_core::copy::protocol::{CopyCommand, CopyEvent, CopyItem, CopyRequest, CopyScope};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::Namespace;
use uuid::Uuid;

use super::copy::CopyInput;

pub(super) const A: u128 = 1;
pub(super) const B: u128 = 2;
/// What the person types to confirm a paste of two collections onto `crm`: the target database.
pub(super) const REQUIRED: &str = "crm";

pub(super) fn sid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

pub(super) fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// `A` (active): shop with orders and users. `B` (parked): crm with contacts.
pub(super) fn base(b_production: bool, b_read_only: bool) -> AppState {
    let mut state = AppState {
        open: vec![sid(A), sid(B)],
        active: Some(sid(A)),
        status: ConnStatus::Connected,
        sidebar: SidebarState::default()
            .with_databases(names(&["shop"]))
            .toggled("shop")
            .with_collections("shop", names(&["orders", "users"])),
        ..AppState::default()
    };
    state.parked.insert(
        sid(B),
        SessionView {
            status: ConnStatus::Connected,
            production: b_production,
            read_only: b_read_only,
            sidebar: SidebarState::default()
                .with_databases(names(&["crm"]))
                .toggled("crm")
                .with_collections("crm", names(&["contacts"])),
            ..SessionView::default()
        },
    );
    state
}

pub(super) fn paste_onto(state: AppState, target: PasteTarget) -> AppState {
    let state = AppState {
        clipboard: CopyClipboard::collections(sid(A), "shop", names(&["orders", "users"])).ok(),
        ..state
    };
    update(state, Input::Sidebar(SidebarAction::Paste(target))).0
}

pub(super) fn onto_crm() -> PasteTarget {
    PasteTarget::Database {
        session: sid(B),
        db: "crm".into(),
    }
}

pub(super) fn input(state: AppState, input: CopyInput) -> (AppState, Vec<Command>) {
    update(state, Input::Copy(input))
}

pub(super) fn event(state: AppState, event: CopyEvent) -> (AppState, Vec<Command>) {
    update(state, Input::Event(Event::Copy(event)))
}

/// A dialog for orders and users pasted onto `crm` of B, opened, with the preview sent.
pub(super) fn opened(b_production: bool) -> (AppState, u64) {
    let s = paste_onto(base(b_production, false), onto_crm());
    let (s, commands) = input(s, CopyInput::Open);
    let run = preview_run(&commands);
    (s, run)
}

pub(super) fn preview_run(commands: &[Command]) -> u64 {
    match commands {
        [Command::Copy(CopyCommand::Preview { run, .. })] => *run,
        other => panic!("expected one Preview, got {other:?}"),
    }
}

pub(super) fn items_of(state: &AppState) -> Vec<CopyItem> {
    match &state.copy.as_ref().unwrap().request.scope {
        CopyScope::Collections(items) => items.clone(),
        other => panic!("{other:?}"),
    }
}

pub(super) fn planned(state: AppState, run: u64) -> AppState {
    let items = items_of(&state);
    event(state, CopyEvent::Planned { run, items }).0
}

pub(super) fn counts(new: u64, changed: u64, unchanged: u64) -> Option<PreviewCounts> {
    Some(PreviewCounts {
        new,
        changed,
        unchanged,
    })
}

/// The preview run finished counting both collections.
pub(super) fn counted(state: AppState, run: u64) -> AppState {
    let s = planned(state, run);
    let (s, _) = event(
        s,
        CopyEvent::PreviewItemDone {
            run,
            index: 0,
            counts: counts(5, 1, 2),
        },
    );
    let (s, _) = event(
        s,
        CopyEvent::PreviewItemDone {
            run,
            index: 1,
            counts: None,
        },
    );
    event(s, CopyEvent::PreviewDone { run }).0
}

pub(super) fn start_run(commands: &[Command]) -> (u64, CopyRequest) {
    match commands {
        [Command::Copy(CopyCommand::Start { run, request })] => (*run, request.clone()),
        other => panic!("expected one Start, got {other:?}"),
    }
}

pub(super) fn dialog(state: &AppState) -> &super::copy_state::CopyDialog {
    state.copy.as_ref().expect("the dialog is open")
}

pub(super) fn typed(state: AppState, text: &str) -> AppState {
    input(state, CopyInput::TypeConfirmation(text.into())).0
}

pub(super) fn with_policy(state: AppState, policy: ConflictPolicy) -> AppState {
    input(state, CopyInput::SetPolicy(policy)).0
}

pub(super) fn report(written: u64, outcome: CopyOutcome) -> CopyReport {
    CopyReport {
        progress: CopyProgress {
            scanned: written,
            written,
            skipped: 0,
            failed: 0,
        },
        failures: Vec::new(),
        outcome,
    }
}

/// Confirmed and started: the state and the run id of the copy.
pub(super) fn started(b_production: bool) -> (AppState, u64) {
    let (s, run) = opened(b_production);
    let s = counted(s, run);
    let s = match b_production {
        true => typed(s, REQUIRED),
        false => s,
    };
    let (s, commands) = input(s, CopyInput::Confirm);
    let (copy_run, _) = start_run(&commands);
    let s = planned(s, copy_run);
    (s, copy_run)
}

pub(super) fn run_of(state: &AppState) -> &super::copy_state::CopyRun {
    match &dialog(state).phase {
        CopyPhase::Running(run) => run,
        other => panic!("{other:?}"),
    }
}

#[test]
fn opening_sends_preview_and_dialog_shows_counting() {
    let s = paste_onto(base(false, false), onto_crm());
    assert!(s.pending_paste.is_some());
    let (s, commands) = input(s, CopyInput::Open);
    let Command::Copy(CopyCommand::Preview { request, .. }) = &commands[0] else {
        panic!("{commands:?}")
    };
    assert_eq!((request.source, request.target), (sid(A), sid(B)));
    assert_eq!(s.pending_paste, None, "the dialog took the request");
    let d = dialog(&s);
    assert!(d.counting);
    assert_eq!(d.counted, 0);
    assert!(matches!(d.phase, CopyPhase::Editing));
}

#[test]
fn opening_without_a_pending_paste_does_nothing() {
    let (s, commands) = input(base(false, false), CopyInput::Open);
    assert!(s.copy.is_none() && commands.is_empty());
}

#[test]
fn default_policy_is_skip_and_options_off() {
    let (s, _) = opened(false);
    let d = dialog(&s);
    assert_eq!(d.policy, ConflictPolicy::Skip);
    assert!(!d.with_indexes_and_options);
    assert_eq!(
        d.names,
        vec![
            ("orders".to_string(), "orders".to_string()),
            ("users".to_string(), "users".to_string())
        ]
    );
    let (s, commands) = input(s, CopyInput::ToggleIndexesAndOptions);
    assert!(commands.is_empty());
    assert!(dialog(&s).with_indexes_and_options);
}

#[test]
fn counts_arrive_per_collection_and_counting_ends_on_preview_done() {
    let (s, run) = opened(false);
    let s = planned(s, run);
    let (s, _) = event(
        s,
        CopyEvent::PreviewProgress {
            run,
            index: 0,
            scanned: 40,
        },
    );
    assert_eq!((dialog(&s).scanned, dialog(&s).counted), (40, 0));
    let (s, _) = event(
        s,
        CopyEvent::PreviewItemDone {
            run,
            index: 0,
            counts: counts(5, 1, 2),
        },
    );
    let d = dialog(&s);
    assert_eq!((d.counted, d.scanned, d.counting), (1, 0, true));
    assert_eq!(d.counts[0], counts(5, 1, 2));
    let (s, _) = event(
        s,
        CopyEvent::PreviewItemDone {
            run,
            index: 1,
            counts: None,
        },
    );
    let (s, _) = event(s, CopyEvent::PreviewDone { run });
    let d = dialog(&s);
    assert!(!d.counting);
    assert_eq!((d.counted, d.counts[1]), (2, None), "unavailable is None");
    assert!(d.preview_run.is_none());
}

#[test]
fn counts_of_another_run_are_ignored() {
    let (s, run) = opened(false);
    let (s, _) = event(
        s,
        CopyEvent::PreviewItemDone {
            run: run + 77,
            index: 0,
            counts: counts(1, 1, 1),
        },
    );
    assert_eq!(dialog(&s).counted, 0);
}

#[test]
fn unavailable_counts_keep_copy_enabled() {
    let (s, run) = opened(false);
    let s = counted(s, run);
    assert!(dialog(&s).counts.iter().any(Option::is_none));
    assert!(dialog(&s).can_confirm(false));
}

#[test]
fn confirm_is_enabled_while_counting_and_says_counts_are_incomplete() {
    let (s, run) = opened(false);
    let s = planned(s, run);
    assert!(dialog(&s).counting && dialog(&s).can_confirm(false));
    let (s, commands) = input(s, CopyInput::Confirm);
    // The preview holds the one run slot: it is cancelled first, the copy follows its end.
    assert!(matches!(
        commands.as_slice(),
        [Command::Copy(CopyCommand::Cancel { run: r })] if *r == run
    ));
    assert!(matches!(dialog(&s).phase, CopyPhase::Starting));
    let (s, commands) = event(
        s,
        CopyEvent::Done {
            run,
            cancelled: true,
        },
    );
    let (copy_run, request) = start_run(&commands);
    assert_ne!(copy_run, run);
    assert_eq!(request.policy, ConflictPolicy::Skip);
    assert!(run_of(&s).counts_incomplete);
}

#[test]
fn confirm_after_counting_does_not_flag_incomplete_counts() {
    let (s, copy_run) = started(false);
    let _ = copy_run;
    assert!(!run_of(&s).counts_incomplete);
}

#[test]
fn skip_and_upsert_need_no_typed_word_unless_target_is_production() {
    for policy in [ConflictPolicy::Skip, ConflictPolicy::Upsert] {
        let s = with_policy(counted_dialog(false), policy);
        let d = dialog(&s);
        assert!(!d.needs_typed_confirmation(false), "{policy:?}");
        assert!(d.needs_typed_confirmation(true), "{policy:?} on production");
        assert!(d.can_confirm(false), "{policy:?}");
    }
}

/// A dialog whose preview finished counting.
fn counted_dialog(b_production: bool) -> AppState {
    let (s, run) = opened(b_production);
    counted(s, run)
}

#[test]
fn production_target_needs_the_typed_word_to_send_start() {
    let (s, commands) = input(counted_dialog(true), CopyInput::Confirm);
    assert!(commands.is_empty(), "nothing is sent without the word");
    assert!(matches!(dialog(&s).phase, CopyPhase::Editing));
    let s = typed(s, "cop");
    let (s, commands) = input(s, CopyInput::Confirm);
    assert!(commands.is_empty());
    let s = typed(s, "  crm ");
    let (_, commands) = input(s, CopyInput::Confirm);
    assert!(matches!(
        commands.as_slice(),
        [Command::Copy(CopyCommand::Start { .. })]
    ));
}

#[test]
fn overwrite_needs_the_typed_word() {
    let (s, run) = opened(false);
    let s = with_policy(counted(s, run), ConflictPolicy::Overwrite);
    assert!(dialog(&s).needs_typed_confirmation(false));
    assert!(!dialog(&s).can_confirm(false));
    let (s, commands) = input(s, CopyInput::Confirm);
    assert!(commands.is_empty());
    let s = typed(s, REQUIRED);
    assert!(dialog(&s).can_confirm(false));
    let (_, commands) = input(s, CopyInput::Confirm);
    let (_, request) = start_run(&commands);
    assert_eq!(request.policy, ConflictPolicy::Overwrite);
}

#[test]
fn confirm_sends_start_only_when_the_dialog_allows() {
    // No dialog: nothing.
    let (s, commands) = input(base(false, false), CopyInput::Confirm);
    assert!(s.copy.is_none() && commands.is_empty());
    // A target that became read-only: nothing is sent, the dialog says why.
    let (s, run) = opened(false);
    let mut s = counted(s, run);
    s.parked.get_mut(&sid(B)).unwrap().read_only = true;
    let (s, commands) = input(s, CopyInput::Confirm);
    assert!(commands.is_empty());
    let run = run_of(&s);
    assert_eq!(run.outcome, Some(RunOutcome::Failed(CoreError::ReadOnly)));
    // A second Confirm on a running copy sends nothing either.
    let (s, _) = started(false);
    assert!(input(s, CopyInput::Confirm).1.is_empty());
}

#[test]
fn the_request_sent_carries_policy_options_and_edited_names() {
    let (s, run) = opened(false);
    let s = counted(s, run);
    let s = with_policy(s, ConflictPolicy::Upsert);
    let (s, _) = input(s, CopyInput::ToggleIndexesAndOptions);
    let (s, commands) = input(
        s,
        CopyInput::SetName {
            index: 0,
            name: "orders_2".into(),
        },
    );
    // The edit cancelled nothing (the preview was done) and asked for fresh counts.
    let fresh = preview_run(&commands);
    let s = counted(s, fresh);
    let (_, commands) = input(s, CopyInput::Confirm);
    let (_, request) = start_run(&commands);
    assert!(request.with_indexes_and_options);
    assert_eq!(request.policy, ConflictPolicy::Upsert);
    let CopyScope::Collections(items) = request.scope else {
        panic!()
    };
    assert_eq!(items[0].target, Namespace::new("crm", "orders_2"));
    assert_eq!(items[0].source, Namespace::new("shop", "orders"));
    assert_eq!(items[1].target, Namespace::new("crm", "users"));
}

#[test]
fn renaming_while_counting_cancels_then_recounts_with_the_latest_name() {
    let (s, run) = opened(false);
    let s = planned(s, run);
    let (s, commands) = input(
        s,
        CopyInput::SetName {
            index: 0,
            name: "o".into(),
        },
    );
    assert!(matches!(
        commands.as_slice(),
        [Command::Copy(CopyCommand::Cancel { run: r })] if *r == run
    ));
    let (s, commands) = input(
        s,
        CopyInput::SetName {
            index: 0,
            name: "ord".into(),
        },
    );
    assert!(commands.is_empty(), "one restart is pending, not two");
    assert_eq!(dialog(&s).counted, 0, "old counts are gone");
    let (s, commands) = event(
        s,
        CopyEvent::Done {
            run,
            cancelled: true,
        },
    );
    let Command::Copy(CopyCommand::Preview { request, .. }) = &commands[0] else {
        panic!("{commands:?}")
    };
    let CopyScope::Collections(items) = &request.scope else {
        panic!()
    };
    assert_eq!(items[0].target.collection, "ord");
    assert!(dialog(&s).counting);
}

#[test]
fn invalid_or_duplicate_target_names_block_confirm() {
    let s = counted_dialog(false);
    for (name, issue) in [
        (
            "",
            NameIssue::Invalid(curlybrace_core::copy::names::NameProblem::Empty),
        ),
        (
            "a$b",
            NameIssue::Invalid(curlybrace_core::copy::names::NameProblem::ForbiddenChar),
        ),
        (
            "system.x",
            NameIssue::Invalid(curlybrace_core::copy::names::NameProblem::SystemPrefix),
        ),
        ("users", NameIssue::Duplicate),
    ] {
        let (blocked, _) = input(
            counted_dialog(false),
            CopyInput::SetName {
                index: 0,
                name: name.into(),
            },
        );
        assert_eq!(dialog(&blocked).name_issue(0), Some(issue), "{name:?}");
        assert!(!dialog(&blocked).can_confirm(false), "{name:?}");
        let (blocked, commands) = input(blocked, CopyInput::Confirm);
        assert!(commands.is_empty(), "{name:?}");
        assert!(matches!(dialog(&blocked).phase, CopyPhase::Editing));
    }
    // A fixed name unblocks it.
    let (fixed, _) = input(
        s,
        CopyInput::SetName {
            index: 0,
            name: "orders2".into(),
        },
    );
    assert_eq!(dialog(&fixed).name_issue(0), None);
    assert_eq!(dialog(&fixed).name_problem(0), None);
}

#[test]
fn an_invalid_name_asks_for_no_counts() {
    let (s, run) = opened(false);
    let s = counted(s, run);
    let (s, commands) = input(
        s,
        CopyInput::SetName {
            index: 1,
            name: "".into(),
        },
    );
    assert!(commands.is_empty());
    assert!(!dialog(&s).counting);
}

#[test]
fn pasting_next_to_the_source_suggests_copy_names_and_blocks_the_source_name() {
    let s = paste_onto(
        base(false, false),
        PasteTarget::Database {
            session: sid(A),
            db: "shop".into(),
        },
    );
    let (s, _) = input(s, CopyInput::Open);
    assert_eq!(dialog(&s).names[0], ("orders".into(), "orders_copy".into()));
    let (s, _) = input(
        s,
        CopyInput::SetName {
            index: 0,
            name: "orders".into(),
        },
    );
    assert_eq!(dialog(&s).name_issue(0), Some(NameIssue::SameAsSource));
    assert!(!dialog(&s).can_confirm(false));
}

#[test]
fn a_database_paste_names_the_target_database_and_rows_come_from_planned() {
    let clipboard = CopyClipboard::database(sid(A), "shop").ok();
    let s = AppState {
        clipboard,
        ..base(false, false)
    };
    let s = update(
        s,
        Input::Sidebar(SidebarAction::Paste(PasteTarget::Connection {
            session: sid(B),
        })),
    )
    .0;
    let (s, commands) = input(s, CopyInput::Open);
    let run = preview_run(&commands);
    assert_eq!(
        dialog(&s).names,
        vec![("shop".to_string(), "shop".to_string())]
    );
    let items = vec![
        CopyItem {
            source: Namespace::new("shop", "orders"),
            target: Namespace::new("shop", "orders"),
            filter: Default::default(),
        },
        CopyItem {
            source: Namespace::new("shop", "users"),
            target: Namespace::new("shop", "users"),
            filter: Default::default(),
        },
    ];
    let (s, _) = event(s, CopyEvent::Planned { run, items });
    assert_eq!(dialog(&s).items.len(), 2);
    assert_eq!(dialog(&s).counts.len(), 2);
    let (s, _) = input(
        s,
        CopyInput::SetName {
            index: 0,
            name: "bad name/".into(),
        },
    );
    assert!(matches!(
        dialog(&s).name_issue(0),
        Some(NameIssue::Invalid(_))
    ));
}

#[test]
fn cancel_in_the_dialog_closes_it_and_stops_the_preview() {
    let (s, run) = opened(false);
    let (s, commands) = input(s, CopyInput::Cancel);
    assert!(s.copy.is_none());
    assert!(matches!(
        commands.as_slice(),
        [Command::Copy(CopyCommand::Cancel { run: r })] if *r == run
    ));
    // Late events of the closed dialog change nothing.
    let (s, commands) = event(s, CopyEvent::PreviewDone { run });
    assert!(s.copy.is_none() && commands.is_empty());
}

#[test]
fn a_paste_while_the_dialog_is_open_is_ignored() {
    let (s, _) = opened(false);
    let again = paste_onto(s, onto_crm());
    assert_eq!(again.pending_paste, None);
    assert!(again.copy.is_some());
}

fn overwriting(state: AppState) -> AppState {
    with_policy(state, ConflictPolicy::Overwrite)
}

#[test]
fn the_required_text_is_the_target_collection_for_a_single_collection() {
    let s = AppState {
        clipboard: CopyClipboard::collections(sid(A), "shop", names(&["orders"])).ok(),
        ..base(false, false)
    };
    let s = update(s, Input::Sidebar(SidebarAction::Paste(onto_crm()))).0;
    let (s, _) = input(s, CopyInput::Open);
    let s = overwriting(s);
    assert_eq!(dialog(&s).required_text(), "orders");
    // A renamed target is what has to be typed.
    let (s, _) = input(
        s,
        CopyInput::SetName {
            index: 0,
            name: "orders2".into(),
        },
    );
    assert_eq!(dialog(&s).required_text(), "orders2");
    assert!(!dialog(&typed(s, "orders")).can_confirm(false));
}

#[test]
fn the_required_text_is_the_target_database_for_several_collections() {
    let (s, _) = opened(false);
    assert_eq!(dialog(&s).required_text(), "crm");
    let s = typed(overwriting(s), "crm");
    assert!(dialog(&s).can_confirm(false));
}

#[test]
fn the_required_text_is_the_target_database_for_a_database_paste() {
    let s = AppState {
        clipboard: CopyClipboard::database(sid(A), "shop").ok(),
        ..base(false, false)
    };
    let s = update(
        s,
        Input::Sidebar(SidebarAction::Paste(PasteTarget::Connection {
            session: sid(B),
        })),
    )
    .0;
    let (s, _) = input(s, CopyInput::Open);
    assert_eq!(dialog(&s).required_text(), "shop");
    let (s, _) = input(
        s,
        CopyInput::SetName {
            index: 0,
            name: "shop2".into(),
        },
    );
    assert_eq!(dialog(&s).required_text(), "shop2");
}

#[test]
fn compare_and_copy_share_one_matching_rule() {
    use crate::viewmodel::typed_text_matches;
    assert!(typed_text_matches("crm", "  crm "));
    assert!(!typed_text_matches("crm", "CRM"));
    assert!(
        !typed_text_matches("", ""),
        "an empty requirement never passes"
    );
}

#[test]
fn a_name_equal_to_another_collections_source_name_is_flagged() {
    // orders -> users, while users is itself copied from the same database of the same session.
    let s = paste_onto(
        base(false, false),
        PasteTarget::Database {
            session: sid(A),
            db: "shop".into(),
        },
    );
    let (s, _) = input(s, CopyInput::Open);
    let (s, _) = input(
        s,
        CopyInput::SetName {
            index: 0,
            name: "users".into(),
        },
    );
    assert_eq!(
        dialog(&s).name_issue(0),
        Some(NameIssue::CollidesWithSource)
    );
    assert!(!dialog(&s).can_confirm(false));
    // On another connection the same name is just the conflict policy's business.
    let (s, run) = opened(false);
    let s = counted(s, run);
    let (s, _) = input(
        s,
        CopyInput::SetName {
            index: 0,
            name: "users2".into(),
        },
    );
    assert_eq!(dialog(&s).name_issue(0), None);
}
