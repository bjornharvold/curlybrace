//! The verdict badge row in the accesskit tree, rendered with the REAL English locale: every
//! tone shows an icon plus resolved text (never a missing-key marker), the "why" details open
//! on demand without moving what is below, and the copy button has a name. Fast, offline.
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::{Reason, SuggestedIndex, Verdict, VerdictKind};
use curlybrace_core::store::{Namespace, Query};
use curlybrace_ui::I18n;
use curlybrace_ui::app::VerdictState;
use curlybrace_ui::viewmodel::verdict::{BadgeModel, badge};
use curlybrace_ui::views::create_index::CreateIndex;
use curlybrace_ui::views::verdict_badge::{VerdictAction, show_verdict_row};
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const COMMAND: &str =
    "db.getSiblingDB(\"shop\").getCollection(\"orders\").createIndex({ \"status\": 1 })";

struct Frame {
    model: Option<BadgeModel>,
    create: CreateIndex,
    seen: Vec<VerdictAction>,
    /// Where the next widget (the documents list) would start.
    below_y: f32,
}

fn verdict(kind: VerdictKind, reasons: Vec<Reason>, suggested: bool) -> Verdict {
    Verdict {
        kind,
        index_name: None,
        keys_examined: 0,
        docs_examined: 0,
        returned: 0,
        plan_only: false,
        reasons,
        suggestion: suggested.then(|| SuggestedIndex {
            keys: vec![("status".into(), 1)],
        }),
        namespace: Some("shop.orders".into()),
    }
}

fn ready(v: Verdict) -> VerdictState {
    VerdictState::Ready {
        query: Query::default(),
        verdict: v,
    }
}

fn scan() -> Verdict {
    verdict(
        VerdictKind::CollectionScan,
        vec![Reason::new("collscan")],
        true,
    )
}

fn open(state: &VerdictState) -> Harness<'static, Frame> {
    open_with(state, CreateIndex::On)
}

fn open_with(state: &VerdictState, create: CreateIndex) -> Harness<'static, Frame> {
    let model = badge(state, Some(&Namespace::new("shop", "orders")));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 400.0))
        .build_ui_state(
            |ui, frame: &mut Frame| {
                let action =
                    show_verdict_row(ui, frame.model.as_ref(), frame.create, &I18n::english());
                frame.seen.extend(action);
                frame.below_y = ui.next_widget_position().y;
                ui.label("documents");
            },
            Frame {
                model,
                create,
                seen: Vec::new(),
                below_y: 0.0,
            },
        );
    harness.run_steps(4);
    harness
}

fn expand(harness: &mut Harness<'static, Frame>) {
    harness.get_by_role_and_label(Role::Button, "Why").click();
    harness.run_steps(4);
}

/// Every label and value in the accessibility tree.
fn texts(harness: &Harness<'static, Frame>) -> Vec<String> {
    harness
        .root()
        .children_recursive()
        .flat_map(|node| {
            let n = node.accesskit_node();
            [n.label(), n.value()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
        })
        .collect()
}

fn assert_shows(harness: &Harness<'static, Frame>, expected: &[&str]) {
    let all = texts(harness).join("\n");
    assert!(!all.contains('\u{27e6}'), "missing key marker in:\n{all}");
    for want in expected {
        assert!(all.contains(want), "`{want}` not in:\n{all}");
    }
}

#[test]
fn good_tones_show_icon_and_title() {
    for (kind, title) in [
        (VerdictKind::IndexUsed, "Uses an index"),
        (VerdictKind::CoveredByIndex, "Covered by an index"),
    ] {
        let harness = open(&ready(verdict(kind, vec![], false)));
        assert_shows(&harness, &[&format!("\u{2714} {title}")]);
    }
}

#[test]
fn warn_shows_its_title_and_the_why_opens_on_demand() {
    let mut harness = open(&ready(scan()));
    assert_shows(
        &harness,
        &["\u{26a0} Collection scan: no index used", "Why"],
    );
    assert!(!texts(&harness).join("\n").contains("every document"));
    expand(&mut harness);
    assert_shows(
        &harness,
        &[
            "reads every document in the collection",
            "Suggested index",
            COMMAND,
        ],
    );
    harness
        .get_by_role_and_label(Role::Button, "Copy index command")
        .click();
    harness.run_steps(4);
    assert_eq!(
        harness.state().seen,
        vec![VerdictAction::CopySuggestion(COMMAND.into())]
    );
}

#[test]
fn why_popup_offers_create_index_beside_copy() {
    let mut harness = open(&ready(scan()));
    expand(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Copy index command");
    let create = harness.get_by_role_and_label(Role::Button, "Create index...");
    assert!(!create.accesskit_node().is_disabled());
    create.click();
    harness.run_steps(4);
    assert_eq!(harness.state().seen, vec![VerdictAction::CreateIndex]);
}

#[test]
fn create_index_is_off_with_its_reason_when_read_only_or_busy() {
    let i18n = I18n::english();
    for (create, key) in [
        (CreateIndex::ReadOnly, "indexes.create_suggested_read_only"),
        (CreateIndex::IndexWrite, "indexes.write_busy"),
        (CreateIndex::Prompt, "indexes.create_prompt_open"),
        (CreateIndex::WriteWarning, "indexes.create_warning_open"),
    ] {
        let mut harness = open_with(&ready(scan()), create);
        expand(&mut harness);
        let reason = i18n.t(key);
        let button = harness.get_by_role_and_label(Role::Button, "Create index...");
        assert!(button.accesskit_node().is_disabled());
        assert_eq!(
            button.accesskit_node().description().as_deref(),
            Some(reason.as_str())
        );
        button.click();
        harness.run_steps(4);
        assert!(harness.state().seen.is_empty(), "{key}");
    }
}

#[test]
fn no_suggestion_means_no_create_index() {
    let mut harness = open(&ready(verdict(
        VerdictKind::CollectionScan,
        vec![Reason::new("collscan")],
        false,
    )));
    expand(&mut harness);
    assert!(harness.query_by_label("Create index...").is_none());
}

#[test]
fn a_hidden_create_index_is_not_drawn() {
    let mut harness = open_with(&ready(scan()), CreateIndex::Hidden);
    expand(&mut harness);
    harness.get_by_role_and_label(Role::Button, "Copy index command");
    assert!(harness.query_by_label("Create index...").is_none());
}

#[test]
fn the_inefficient_reason_carries_its_counts() {
    let reason = Reason::new("inefficient")
        .with_param("examined", 500)
        .with_param("returned", 3);
    let mut harness = open(&ready(verdict(
        VerdictKind::InefficientIndex,
        vec![reason],
        false,
    )));
    expand(&mut harness);
    assert_shows(&harness, &["examined 500 entries to return 3 documents"]);
}

#[test]
fn neutral_tones_show_their_reasons() {
    for (kind, key, title, reason) in [
        (VerdictKind::NoFilter, "no_filter", "No filter", "no filter"),
        (
            VerdictKind::EmptyCollection,
            "empty_collection",
            "Collection is empty",
            "empty",
        ),
    ] {
        let mut harness = open(&ready(verdict(kind, vec![Reason::new(key)], false)));
        assert_shows(&harness, &[&format!("\u{2139} {title}")]);
        expand(&mut harness);
        assert_shows(&harness, &[reason]);
    }
}

#[test]
fn plan_only_shows_the_estimate_note() {
    let mut v = scan();
    v.plan_only = true;
    let mut harness = open(&ready(v));
    expand(&mut harness);
    assert_shows(&harness, &["Estimated from the query plan only"]);
}

#[test]
fn unavailable_states_a_fixed_reason_and_never_the_server_text() {
    for (error, reason) in [
        (
            CoreError::Unsupported("secret-detail".into()),
            "server or store cannot explain this query",
        ),
        (
            CoreError::Unauthorized,
            "not allowed to explain queries here",
        ),
        (
            CoreError::Database("secret-detail".into()),
            "could not explain this query",
        ),
    ] {
        let mut harness = open(&VerdictState::Unavailable {
            query: Query::default(),
            error,
        });
        assert_shows(&harness, &["\u{2139} Index check unavailable"]);
        expand(&mut harness);
        assert_shows(&harness, &[reason]);
        assert!(!texts(&harness).join("\n").contains("secret-detail"));
    }
}

#[test]
fn checking_says_so_and_has_nothing_to_expand() {
    let harness = open(&VerdictState::Pending {
        query: Query::default(),
    });
    assert_shows(&harness, &["\u{2026} Checking how this query runs..."]);
    assert!(harness.query_by_label("Why").is_none());
}

#[test]
fn an_unknown_reason_still_shows_a_visible_fallback() {
    let mut harness = open(&ready(verdict(
        VerdictKind::CollectionScan,
        vec![Reason::new("brand_new")],
        false,
    )));
    expand(&mut harness);
    assert_shows(&harness, &["brand_new"]);
}

#[test]
fn what_is_below_the_row_never_moves() {
    let none = open(&VerdictState::None);
    let reference = none.state().below_y;
    assert!(none.state().model.is_none() && reference > 0.0);
    let pending = open(&VerdictState::Pending {
        query: Query::default(),
    });
    let mut warn = open(&ready(scan()));
    assert_eq!(pending.state().below_y, reference);
    assert_eq!(warn.state().below_y, reference);
    expand(&mut warn);
    assert_eq!(warn.state().below_y, reference);
}
