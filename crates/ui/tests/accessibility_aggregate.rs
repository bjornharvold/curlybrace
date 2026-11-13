//! The Aggregate tab in the accesskit tree, rendered with the REAL English locale: every control
//! has a role and a name (the stage controls carry the stage's position), Tab walks the header
//! and the first stage in reading order, Add stage adds a card, invalid pipeline text shows its
//! line and column and keeps text mode, and the preview pane lists documents, the stages that
//! were not run and a timeout. The draft is driven through the actions the view returns, applied
//! with the draft's own ops as the reducer does. Fast, offline, no GPU.
use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::COMMAND_TIMEOUT_MESSAGE;
use curlybrace_ui::I18n;
use curlybrace_ui::app::AggregateAction;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::AggregateState;
use curlybrace_ui::viewmodel::aggregate::{Mode, PreviewState};
use curlybrace_ui::viewmodel::aggregate_view::AggregateView;
use curlybrace_ui::views::aggregate::show_aggregate;
use eframe::egui;
use eframe::egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

struct Frame {
    draft: AggregateState,
    view: AggregateView,
    seen: Vec<AggregateAction>,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn tn(key: &str, n: usize) -> String {
    I18n::english().t_with(key, &[("n", &n.to_string())])
}

/// `draft` after `action`, by the draft's own ops (the reducer also sends a preview, which these
/// tests leave out: the pane keeps what it showed).
fn applied(draft: AggregateState, action: &AggregateAction) -> AggregateState {
    match action {
        AggregateAction::AddStage(name) => draft.add_stage(name),
        AggregateAction::Remove(i) => draft.remove(*i),
        AggregateAction::MoveUp(i) => draft.move_up(*i),
        AggregateAction::MoveDown(i) => draft.move_down(*i),
        AggregateAction::Toggle(i) => draft.toggle(*i),
        AggregateAction::Edit(i, text) => draft.edit(*i, text),
        AggregateAction::Select(i) => draft.select(*i),
        AggregateAction::SetMode(Mode::Text) => draft.to_text(),
        AggregateAction::SetMode(Mode::Stages) => draft.from_text(),
        AggregateAction::EditText(text) => draft.with_text(text),
        AggregateAction::Refresh => draft,
    }
}

/// `draft` previewed: the selected stage's request answered with `outcome`.
fn previewed(
    draft: AggregateState,
    outcome: Result<Vec<bson::Document>, CoreError>,
) -> AggregateState {
    let (draft, sent) = draft.begin_preview();
    let (request, _) = sent.expect("a valid pipeline is sent");
    draft.arrived(request, outcome)
}

fn two_docs() -> Vec<bson::Document> {
    vec![
        doc! { "_id": 1, "city": "Berlin" },
        doc! { "_id": 2, "city": "Lisbon" },
    ]
}

/// `$match`, `$sort`, `$limit`, the last previewed with two documents.
fn three_stages_shown() -> AggregateState {
    let draft = AggregateState::default()
        .add_stage("$match")
        .add_stage("$sort")
        .add_stage("$limit");
    previewed(draft, Ok(two_docs()))
}

fn open(draft: AggregateState) -> Harness<'static, Frame> {
    open_sized(draft, (1000.0, 800.0))
}

fn open_sized(draft: AggregateState, size: (f32, f32)) -> Harness<'static, Frame> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .build_ui_state(
            move |ui, frame: &mut Frame| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                let actions = show_aggregate(ui, &frame.draft, &mut frame.view, &I18n::english());
                for action in &actions {
                    frame.draft = applied(std::mem::take(&mut frame.draft), action);
                }
                frame.seen.extend(actions);
            },
            Frame {
                draft,
                view: AggregateView::default(),
                seen: Vec::new(),
            },
        );
    harness.run_steps(4);
    harness
}

fn toggled(harness: &Harness<'static, Frame>, role: Role, label: &str) -> Option<Toggled> {
    harness
        .get_by_role_and_label(role, label)
        .accesskit_node()
        .toggled()
}

#[test]
fn every_header_control_has_a_role_and_a_name() {
    let harness = open(three_stages_shown());
    harness.get_by_role_and_label(Role::ComboBox, &t("aggregate.add_stage"));
    harness.get_by_role_and_label(Role::Button, &t("aggregate.refresh"));
    let stages = t("aggregate.mode.stages");
    let text = t("aggregate.mode.text");
    assert_eq!(
        toggled(&harness, Role::RadioButton, &stages),
        Some(Toggled::True)
    );
    assert_eq!(
        toggled(&harness, Role::RadioButton, &text),
        Some(Toggled::False)
    );
    harness.get_by_role_and_label(Role::Label, &tn("aggregate.preview.heading", 3));
    let mut labels = Vec::new();
    for node in harness.root().children_recursive() {
        labels.extend(node.accesskit_node().label());
    }
    assert!(labels.iter().all(|l| !l.contains('\u{27e6}')), "{labels:?}");
}

#[test]
fn every_stage_control_carries_its_position() {
    let harness = open(three_stages_shown());
    for n in 1..=3 {
        let enabled = toggled(&harness, Role::CheckBox, &tn("aggregate.stage.enable_n", n));
        assert_eq!(enabled, Some(Toggled::True), "stage {n}");
        harness.get_by_role_and_label(Role::MultilineTextInput, &tn("aggregate.stage.editor_n", n));
        for key in [
            "aggregate.stage.move_up_n",
            "aggregate.stage.move_down_n",
            "aggregate.stage.remove_n",
        ] {
            harness.get_by_role_and_label(Role::Button, &tn(key, n));
        }
    }
    let disabled = |key: &str, n| {
        harness
            .get_by_role_and_label(Role::Button, &tn(key, n))
            .accesskit_node()
            .is_disabled()
    };
    assert!(
        disabled("aggregate.stage.move_up_n", 1),
        "the first cannot go up"
    );
    assert!(
        disabled("aggregate.stage.move_down_n", 3),
        "the last cannot go down"
    );
    assert!(!disabled("aggregate.stage.move_up_n", 2));
}

#[test]
fn a_disabled_stage_reports_its_unticked_box() {
    let harness = open(three_stages_shown().toggle(1));
    let box2 = toggled(&harness, Role::CheckBox, &tn("aggregate.stage.enable_n", 2));
    assert_eq!(box2, Some(Toggled::False));
    harness.get_by_label(&t("aggregate.stage.disabled"));
}

/// The label of the focused node, if any.
fn focused(harness: &Harness<'static, Frame>) -> Option<String> {
    harness
        .root()
        .children_recursive()
        .find(|n| n.accesskit_node().is_focused())
        .and_then(|n| n.accesskit_node().label())
}

#[test]
fn tab_walks_the_header_then_the_first_stage() {
    let mut harness = open(three_stages_shown());
    harness
        .get_by_role_and_label(Role::ComboBox, &t("aggregate.add_stage"))
        .focus();
    harness.run_steps(2);
    let mut order = vec![focused(&harness)];
    for _ in 0..8 {
        harness.key_press(egui::Key::Tab);
        harness.run_steps(2);
        order.push(focused(&harness));
    }
    let expected: Vec<Option<String>> = [
        t("aggregate.add_stage"),
        t("aggregate.mode.stages"),
        t("aggregate.mode.text"),
        t("aggregate.refresh"),
        tn("aggregate.stage.enable_n", 1),
        tn("aggregate.stage.select_n", 1),
        tn("aggregate.stage.move_down_n", 1),
        tn("aggregate.stage.remove_n", 1),
        tn("aggregate.stage.editor_n", 1),
    ]
    .into_iter()
    .map(Some)
    .collect();
    assert_eq!(order, expected);
}

#[test]
fn add_stage_adds_a_card_for_the_picked_name() {
    let mut harness = open(three_stages_shown());
    harness
        .get_by_role_and_label(Role::ComboBox, &t("aggregate.add_stage"))
        .click();
    harness.run_steps(2);
    harness.get_by_label("$group").click();
    harness.run_steps(4);
    assert!(
        harness
            .state()
            .seen
            .contains(&AggregateAction::AddStage("$group".into()))
    );
    assert_eq!(harness.state().draft.stages.len(), 4);
    harness.get_by_role_and_label(Role::MultilineTextInput, &tn("aggregate.stage.editor_n", 4));
    let valid = I18n::english().t_with("aggregate.stage.valid", &[("name", "$group")]);
    harness.get_by_label(&valid);
}

#[test]
fn remove_and_move_change_the_cards() {
    let mut harness = open(three_stages_shown());
    harness
        .get_by_role_and_label(Role::Button, &tn("aggregate.stage.move_down_n", 1))
        .click();
    harness.run_steps(4);
    let titles = |h: &Harness<'static, Frame>| {
        h.state()
            .draft
            .stages
            .iter()
            .map(|s| s.text.split('"').nth(1).unwrap_or("").to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(titles(&harness), ["$sort", "$match", "$limit"]);
    harness
        .get_by_role_and_label(Role::Button, &tn("aggregate.stage.remove_n", 3))
        .click();
    harness.run_steps(4);
    assert_eq!(titles(&harness), ["$sort", "$match"]);
    assert!(
        harness
            .query_by_label(&tn("aggregate.stage.editor_n", 3))
            .is_none()
    );
}

#[test]
fn invalid_text_shows_line_and_column_and_keeps_text_mode() {
    let draft = three_stages_shown()
        .to_text()
        .with_text("[\n  { \"$match\": \n]");
    let mut harness = open(draft);
    harness.get_by_role_and_label(Role::MultilineTextInput, &t("aggregate.text.editor"));
    harness
        .get_by_role_and_label(Role::RadioButton, &t("aggregate.mode.stages"))
        .click();
    harness.run_steps(4);
    assert_eq!(
        harness.state().draft.mode,
        Mode::Text,
        "invalid text blocks leaving"
    );
    assert!(harness.state().draft.text_error.is_some());
    let error = harness.get_by_label_contains("line 3, column");
    assert_eq!(error.accesskit_node().role(), Role::Label);
    harness.get_by_role_and_label(Role::MultilineTextInput, &t("aggregate.text.editor"));
    assert_eq!(
        toggled(&harness, Role::RadioButton, &t("aggregate.mode.text")),
        Some(Toggled::True)
    );
    assert!(
        harness
            .query_by_label(&tn("aggregate.stage.editor_n", 1))
            .is_none()
    );
}

#[test]
fn the_preview_lists_documents_and_the_stages_not_run() {
    let draft = AggregateState::default()
        .add_stage("$match")
        .add_stage("$out");
    let draft = previewed(draft, Ok(two_docs()));
    assert!(matches!(&draft.preview, PreviewState::Shown(p) if p.not_run == ["$out"]));
    let harness = open(draft);
    harness.get_by_label_contains("Berlin");
    harness.get_by_label_contains("Lisbon");
    let not_run = I18n::english().t_with("aggregate.preview.not_run_one", &[("names", "$out")]);
    harness.get_by_role_and_label(Role::Label, &not_run);
    harness.get_by_label(&tn("aggregate.preview.count_many", 2));
}

#[test]
fn a_timed_out_preview_says_so_and_keeps_the_last_documents() {
    let timeout = Err(CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into()));
    let draft = previewed(three_stages_shown(), timeout);
    assert!(matches!(
        &draft.preview,
        PreviewState::Failed { last: Some(_), .. }
    ));
    let harness = open(draft);
    harness.get_by_role_and_label(Role::Label, &t("aggregate.error.timeout"));
    harness.get_by_label(&t("aggregate.status.failed"));
    harness.get_by_label_contains("Berlin");
    harness.get_by_label_contains("Lisbon");
}
