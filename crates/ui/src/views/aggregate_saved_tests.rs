//! The saved pipeline controls of the Aggregate header and their dialogs, by accessible name and
//! role, on real drafts and a real list. Keyboard: Enter confirms a usable name, Escape cancels,
//! focus starts in the name field (or on Cancel for a prompt that replaces or removes).
use super::aggregate::show_aggregate;
use crate::I18n;
use crate::app::AggregateAction;
use crate::theme;
use crate::viewmodel::aggregate::AggregateState;
use crate::viewmodel::aggregate_saved::SavedDialog;
use crate::viewmodel::aggregate_view::AggregateView;
use curlybrace_core::saved_pipeline::{SavedPipeline, SavedPipelines, SavedStage};
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use uuid::Uuid;

struct Frame {
    draft: AggregateState,
    view: AggregateView,
    saved: Vec<SavedPipeline>,
    seen: Vec<AggregateAction>,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn tn(key: &str, name: &str) -> String {
    I18n::english().t_with(key, &[("name", name)])
}

fn stage() -> Vec<SavedStage> {
    vec![SavedStage {
        text: "{ \"$match\": {} }".into(),
        enabled: true,
    }]
}

/// Pipelines saved in the order given: the last one is the newest and listed first.
fn list_of(names: &[&str]) -> Vec<SavedPipeline> {
    let conn = Uuid::nil();
    let all = names
        .iter()
        .enumerate()
        .fold(SavedPipelines::default(), |all, (i, name)| {
            let hint = Some("crm.customers".to_string());
            all.saved(conn, name, hint, stage(), i as i64, false)
                .expect("a usable name")
        });
    all.list(conn).to_vec()
}

fn open(
    draft: AggregateState,
    saved: Vec<SavedPipeline>,
    size: (f32, f32),
) -> Harness<'static, Frame> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .build_ui_state(
            move |ui, frame: &mut Frame| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                let actions = show_aggregate(
                    ui,
                    &frame.draft,
                    &mut frame.view,
                    &frame.saved,
                    &I18n::english(),
                );
                frame.seen.extend(actions);
            },
            Frame {
                draft,
                view: AggregateView::default(),
                saved,
                seen: Vec::new(),
            },
        );
    harness.run_steps(4);
    harness
}

/// One `$match` stage, previewed (so the tab asks for no automatic refresh).
fn draft() -> AggregateState {
    let (draft, sent) = AggregateState::default()
        .add_stage("$match")
        .begin_preview();
    let (request, _) = sent.expect("a valid pipeline is sent");
    draft.arrived(request, Ok(Vec::new()))
}

fn with_dialog(dialog: SavedDialog) -> AggregateState {
    draft().with_saved_dialog(Some(dialog))
}

fn focused(harness: &Harness<'static, Frame>) -> Option<String> {
    harness
        .root()
        .children_recursive()
        .find(|n| n.accesskit_node().is_focused())
        .and_then(|n| n.accesskit_node().label())
}

/// The validation message the name field is described by (and flagged invalid with).
fn name_field_description(harness: &Harness<'static, Frame>) -> Option<String> {
    let label = t("aggregate.saved.name_label");
    harness
        .get_by_role_and_label(Role::TextInput, &label)
        .accesskit_node()
        .description()
}

const BIG: (f32, f32) = (1000.0, 700.0);

#[test]
fn save_and_open_are_named_buttons() {
    let harness = open(draft(), list_of(&["Top cities"]), BIG);
    harness.get_by_role_and_label(Role::Button, &t("aggregate.saved.save"));
    let label = t("aggregate.saved.open");
    let open_button = harness.get_by_role_and_label(Role::Button, &label);
    assert!(!open_button.accesskit_node().is_disabled());
}

#[test]
fn open_is_off_with_a_reason_when_nothing_is_saved() {
    let harness = open(draft(), Vec::new(), BIG);
    let label = t("aggregate.saved.open");
    let open_button = harness.get_by_role_and_label(Role::Button, &label);
    assert!(open_button.accesskit_node().is_disabled());
    assert_eq!(
        open_button.accesskit_node().description(),
        Some(t("aggregate.saved.open_empty"))
    );
}

#[test]
fn save_click_asks_for_the_dialog() {
    let mut harness = open(draft(), Vec::new(), BIG);
    harness
        .get_by_role_and_label(Role::Button, &t("aggregate.saved.save"))
        .click();
    harness.run_steps(2);
    assert_eq!(harness.state().seen, [AggregateAction::SaveClicked]);
}

#[test]
fn the_open_menu_lists_newest_first_with_rename_and_delete_per_item() {
    let mut harness = open(draft(), list_of(&["Old one", "New one"]), BIG);
    harness
        .get_by_role_and_label(Role::Button, &t("aggregate.saved.open"))
        .click();
    harness.run_steps(3);
    let mut opens: Vec<(f64, String)> = harness
        .get_all_by_role(Role::Button)
        .filter_map(|b| {
            let node = b.accesskit_node();
            Some((node.bounding_box()?.y0, node.label()?))
        })
        .filter(|(_, label)| label.starts_with("Open "))
        .collect();
    opens.sort_by(|a, b| a.0.total_cmp(&b.0));
    let names: Vec<&str> = opens.iter().map(|(_, l)| l.as_str()).collect();
    assert_eq!(names, ["Open New one", "Open Old one"]);
    for name in ["New one", "Old one"] {
        harness.get_by_role_and_label(Role::Button, &tn("aggregate.saved.rename_n", name));
        harness.get_by_role_and_label(Role::Button, &tn("aggregate.saved.delete_n", name));
    }
    assert_eq!(harness.get_all_by_label("crm.customers").count(), 2);
    let id = harness.state().saved[1].id;
    harness
        .get_by_role_and_label(Role::Button, "Open Old one")
        .click();
    harness.run_steps(3);
    assert_eq!(harness.state().seen, [AggregateAction::OpenMenuPicked(id)]);
}

#[test]
fn rename_and_delete_in_the_menu_ask_for_their_dialogs() {
    let mut harness = open(draft(), list_of(&["Top cities"]), BIG);
    let id = harness.state().saved[0].id;
    harness
        .get_by_role_and_label(Role::Button, &t("aggregate.saved.open"))
        .click();
    harness.run_steps(3);
    harness
        .get_by_role_and_label(Role::Button, &tn("aggregate.saved.rename_n", "Top cities"))
        .click();
    harness.run_steps(3);
    assert_eq!(harness.state().seen, [AggregateAction::RenameStarted(id)]);
}

#[test]
fn the_opened_name_and_the_modified_mark_show_in_the_header() {
    let saved = list_of(&["Top cities"]);
    let opened = draft().opened_from(&saved[0]);
    let harness = open(opened.clone(), saved.clone(), BIG);
    harness.get_by_label(&tn("aggregate.saved.opened", "Top cities"));
    assert!(
        harness
            .query_by_label(&t("aggregate.saved.modified"))
            .is_none()
    );
    let edited = opened.edit(0, "{ \"$sort\": { \"a\": 1 } }");
    let harness = open(edited, saved, BIG);
    harness.get_by_label(&t("aggregate.saved.modified"));
}

#[test]
fn a_long_name_is_elided_but_keeps_its_full_accessible_name() {
    let long = "N".repeat(80);
    let saved = list_of(&[&long]);
    let opened = draft().opened_from(&saved[0]);
    let mut harness = open(opened, saved, (360.0, 500.0));
    let full = tn("aggregate.saved.opened", &long);
    harness.get_by_label(&full);
    // What is drawn is shorter than the name and ends in an ellipsis.
    let drawn: Vec<String> = harness
        .output()
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => {
                Some(t.galley.rows.iter().map(|r| r.text()).collect::<String>())
            }
            _ => None,
        })
        .collect();
    assert!(
        !drawn.contains(&full),
        "the whole name was drawn: {drawn:?}"
    );
    assert!(
        drawn.iter().any(|t| t.contains("NN")
            && t.ends_with('…')
            && t.chars().count() < full.chars().count()),
        "no elided name among {drawn:?}"
    );
    // The full name is in the tooltip only once the chip is hovered.
    assert_eq!(harness.get_all_by_label(&full).count(), 1);
    harness.get_by_label(&full).hover();
    harness.run_steps(12);
    assert_eq!(harness.get_all_by_label(&full).count(), 2, "the tooltip");
}

#[test]
fn the_save_dialog_focuses_the_name_and_enter_saves() {
    let dialog = SavedDialog::Save {
        name: "Top cities".into(),
        replace_prompt: false,
    };
    let mut harness = open(with_dialog(dialog), Vec::new(), BIG);
    assert_eq!(focused(&harness), Some(t("aggregate.saved.name_label")));
    harness.get_by_label(&t("aggregate.saved.save_title"));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(harness.state().seen, [AggregateAction::SaveConfirmed]);
}

#[test]
fn typing_in_the_name_field_reports_the_new_name() {
    let dialog = SavedDialog::Save {
        name: String::new(),
        replace_prompt: false,
    };
    let mut harness = open(with_dialog(dialog), Vec::new(), BIG);
    harness
        .get_by_role_and_label(Role::TextInput, &t("aggregate.saved.name_label"))
        .type_text("Q1");
    harness.run_steps(3);
    assert!(
        harness
            .state()
            .seen
            .contains(&AggregateAction::SaveNameEdited("Q1".into()))
    );
}

#[test]
fn escape_cancels_the_dialog() {
    let dialog = SavedDialog::Save {
        name: "Top".into(),
        replace_prompt: false,
    };
    let mut harness = open(with_dialog(dialog), Vec::new(), BIG);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert_eq!(harness.state().seen, [AggregateAction::DialogCancelled]);
}

#[test]
fn an_empty_name_cannot_be_saved_and_enter_does_nothing() {
    let dialog = SavedDialog::Save {
        name: "  ".into(),
        replace_prompt: false,
    };
    let mut harness = open(with_dialog(dialog), Vec::new(), BIG);
    let label = t("aggregate.saved.save");
    let confirm = harness.get_all_by_role_and_label(Role::Button, &label);
    assert!(
        confirm
            .into_iter()
            .any(|b| b.accesskit_node().is_disabled())
    );
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert!(
        harness.state().seen.is_empty(),
        "{:?}",
        harness.state().seen
    );
}

#[test]
fn a_name_over_80_characters_shows_its_message_and_cannot_be_saved() {
    let dialog = SavedDialog::Save {
        name: "x".repeat(81),
        replace_prompt: false,
    };
    let mut harness = open(with_dialog(dialog), Vec::new(), BIG);
    assert_eq!(
        name_field_description(&harness),
        Some(t("aggregate.saved.error.name_too_long"))
    );
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert!(harness.state().seen.is_empty());
}

#[test]
fn a_name_with_a_line_break_shows_its_message_and_cannot_be_saved() {
    let dialog = SavedDialog::Save {
        name: "two\nlines".into(),
        replace_prompt: false,
    };
    let mut harness = open(with_dialog(dialog), Vec::new(), BIG);
    assert_eq!(
        name_field_description(&harness),
        Some(t("aggregate.saved.error.invalid_name"))
    );
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert!(harness.state().seen.is_empty());
}

#[test]
fn the_rename_dialog_refuses_a_name_another_pipeline_has_but_not_its_own() {
    let saved = list_of(&["Alpha", "Beta"]);
    let beta = saved[0].id;
    let taken = SavedDialog::Rename {
        id: beta,
        name: " alpha ".into(),
    };
    let harness = open(with_dialog(taken), saved.clone(), BIG);
    assert_eq!(
        name_field_description(&harness),
        Some(t("aggregate.saved.error.name_taken"))
    );
    let own = SavedDialog::Rename {
        id: beta,
        name: "BETA".into(),
    };
    let mut harness = open(with_dialog(own), saved, BIG);
    assert_eq!(name_field_description(&harness), None);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(harness.state().seen, [AggregateAction::RenameConfirmed]);
}

#[test]
fn a_prompt_that_replaces_or_removes_starts_on_cancel_so_enter_changes_nothing() {
    let saved = list_of(&["Top cities"]);
    let id = saved[0].id;
    let dialogs = [
        SavedDialog::Save {
            name: "Top cities".into(),
            replace_prompt: true,
        },
        SavedDialog::ConfirmOpen { id },
        SavedDialog::ConfirmDelete { id },
    ];
    for dialog in dialogs {
        let mut harness = open(with_dialog(dialog.clone()), saved.clone(), BIG);
        assert_eq!(focused(&harness), Some(t("common.cancel")), "{dialog:?}");
        harness.key_press(egui::Key::Enter);
        harness.run_steps(3);
        assert_eq!(harness.state().seen, [AggregateAction::DialogCancelled]);
    }
}

#[test]
fn the_replace_prompt_names_the_pipeline_and_confirms_with_replace() {
    let dialog = SavedDialog::Save {
        name: " top CITIES ".into(),
        replace_prompt: true,
    };
    // The message names the stored pipeline, spelled as it is stored, not what was typed.
    let mut harness = open(with_dialog(dialog), list_of(&["Top cities"]), BIG);
    harness.get_by_label(&tn("aggregate.saved.replace_message", "Top cities"));
    harness
        .get_by_role_and_label(Role::Button, &t("aggregate.saved.replace_confirm"))
        .click();
    harness.run_steps(3);
    assert_eq!(harness.state().seen, [AggregateAction::SaveConfirmed]);
}

#[test]
fn the_confirm_buttons_of_the_prompts_send_their_action() {
    let saved = list_of(&["Top cities"]);
    let id = saved[0].id;
    let cases = [
        (
            SavedDialog::ConfirmOpen { id },
            "aggregate.saved.open_confirm",
            AggregateAction::OpenConfirmed,
        ),
        (
            SavedDialog::ConfirmDelete { id },
            "aggregate.saved.delete_confirm",
            AggregateAction::DeleteConfirmed,
        ),
    ];
    for (dialog, key, expected) in cases {
        let mut harness = open(with_dialog(dialog), saved.clone(), BIG);
        harness.get_by_role_and_label(Role::Button, &t(key)).click();
        harness.run_steps(3);
        assert_eq!(harness.state().seen, [expected]);
    }
}

#[test]
fn a_dialog_for_a_pipeline_that_is_gone_cancels_itself() {
    let dialog = SavedDialog::ConfirmDelete { id: Uuid::new_v4() };
    let harness = open(with_dialog(dialog), list_of(&["Top cities"]), BIG);
    assert!(
        harness
            .state()
            .seen
            .contains(&AggregateAction::DialogCancelled)
    );
}

#[test]
fn a_failed_confirm_is_shown_in_the_dialog() {
    let dialog = SavedDialog::Save {
        name: "Top".into(),
        replace_prompt: false,
    };
    let failed = with_dialog(dialog).with_saved_error("aggregate.saved.write_failed");
    let harness = open(failed, Vec::new(), BIG);
    harness.get_by_label(&t("aggregate.saved.write_failed"));
}

#[test]
fn typing_that_waits_is_sent_before_the_save_click() {
    let mut harness = open(draft(), Vec::new(), BIG);
    let (view, _) =
        std::mem::take(&mut harness.state_mut().view).edited(0, "{ \"$limit\": 5 }".into(), 0.0);
    harness.state_mut().view = view;
    harness
        .get_by_role_and_label(Role::Button, &t("aggregate.saved.save"))
        .click();
    harness.run_steps(3);
    let seen = &harness.state().seen;
    assert_eq!(
        seen.first(),
        Some(&AggregateAction::Edit(0, "{ \"$limit\": 5 }".into())),
        "{seen:?}"
    );
    assert!(seen.contains(&AggregateAction::SaveClicked));
}
