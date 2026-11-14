//! The saved pipeline controls of the Aggregate tab in the accesskit tree, rendered with the
//! REAL English locale at the standard 1000x700 and the small 800x500 window: Save, Open and the
//! opened pipeline's name have roles and names and stay inside the window without overlapping;
//! the Open menu and every dialog fit the small window with named controls; Tab walks a dialog in
//! reading order and a prompt that replaces or removes starts on Cancel. Fast, offline, no GPU.
use curlybrace_core::saved_pipeline::{SavedPipeline, SavedPipelines, SavedStage};
use curlybrace_ui::I18n;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::AggregateState;
use curlybrace_ui::viewmodel::aggregate_saved::SavedDialog;
use curlybrace_ui::viewmodel::aggregate_view::AggregateView;
use curlybrace_ui::views::aggregate::show_aggregate;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use uuid::Uuid;

const STANDARD: (f32, f32) = (1000.0, 700.0);
const SMALL: (f32, f32) = (800.0, 500.0);

type Frame = (AggregateState, AggregateView, Vec<SavedPipeline>);

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn tn(key: &str, name: &str) -> String {
    I18n::english().t_with(key, &[("name", name)])
}

/// Pipelines saved in order; the last is the newest and comes first.
fn saved_list(names: &[&str]) -> Vec<SavedPipeline> {
    let stages = vec![SavedStage {
        text: "{ \"$match\": {} }".into(),
        enabled: true,
    }];
    let all = names
        .iter()
        .enumerate()
        .fold(SavedPipelines::default(), |all, (i, name)| {
            let hint = Some("crm.customers".to_string());
            all.saved(Uuid::nil(), name, hint, stages.clone(), i as i64, false)
                .expect("a usable name")
        });
    all.list(Uuid::nil()).to_vec()
}

/// A previewed one-stage draft (so no automatic refresh is asked for).
fn draft() -> AggregateState {
    let (draft, sent) = AggregateState::default()
        .add_stage("$match")
        .begin_preview();
    let (request, _) = sent.expect("a valid pipeline is sent");
    draft.arrived(request, Ok(Vec::new()))
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
            move |ui, (draft, view, saved): &mut Frame| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                show_aggregate(ui, draft, view, saved, &I18n::english());
            },
            (draft, AggregateView::default(), saved),
        );
    harness.run_steps(4);
    harness
}

fn window(size: (f32, f32)) -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1))
}

fn focused(harness: &Harness<'static, Frame>) -> Option<String> {
    harness
        .root()
        .children_recursive()
        .find(|n| n.accesskit_node().is_focused())
        .and_then(|n| n.accesskit_node().label())
}

/// Every node with a role in `roles` is inside `window`, not squashed, and none overlap.
fn assert_laid_out(harness: &Harness<'static, Frame>, size: (f32, f32), roles: &[Role]) {
    let rects: Vec<(String, egui::Rect)> = harness
        .root()
        .children_recursive()
        .filter(|n| roles.contains(&n.accesskit_node().role()))
        .filter_map(|n| Some((n.accesskit_node().label()?, n.rect())))
        .collect();
    assert!(!rects.is_empty());
    for (label, rect) in &rects {
        assert!(
            rect.width() > 1.0 && rect.height() > 1.0,
            "{label}: {rect:?}"
        );
        assert!(
            window(size).contains_rect(*rect),
            "{label} leaves {size:?}: {rect:?}"
        );
    }
    for (i, (a, ra)) in rects.iter().enumerate() {
        for (b, rb) in &rects[i + 1..] {
            let overlap = ra.intersect(*rb);
            assert!(
                !overlap.is_positive() || overlap.area() < 1.0,
                "{a} {ra:?} overlaps {b} {rb:?}"
            );
        }
    }
}

fn no_placeholders(harness: &Harness<'static, Frame>) {
    for node in harness.root().children_recursive() {
        let label = node.accesskit_node().label().unwrap_or_default();
        assert!(!label.contains('\u{27e6}'), "{label}");
    }
}

#[test]
fn the_toolbar_is_named_and_fits_both_window_sizes() {
    for size in [STANDARD, SMALL] {
        let saved = saved_list(&["Customers per city"]);
        let opened = draft().opened_from(&saved[0]).edit(0, "{ \"$limit\": 3 }");
        let harness = open(opened, saved, size);
        for key in ["aggregate.saved.save", "aggregate.saved.open"] {
            harness.get_by_role_and_label(Role::Button, &t(key));
        }
        harness.get_by_label(&tn("aggregate.saved.opened", "Customers per city"));
        harness.get_by_label(&t("aggregate.saved.modified"));
        let mut rects = Vec::new();
        for (role, label) in [
            (Role::Button, t("aggregate.saved.save")),
            (Role::Button, t("aggregate.saved.open")),
            (
                Role::Label,
                tn("aggregate.saved.opened", "Customers per city"),
            ),
            (Role::Label, t("aggregate.saved.modified")),
            (Role::Button, t("aggregate.refresh")),
            (Role::ComboBox, t("aggregate.add_stage")),
        ] {
            rects.push((
                label.clone(),
                harness.get_by_role_and_label(role, &label).rect(),
            ));
        }
        for (label, rect) in &rects {
            assert!(
                window(size).contains_rect(*rect),
                "{label} {size:?}: {rect:?}"
            );
        }
        for (i, (a, ra)) in rects.iter().enumerate() {
            for (b, rb) in &rects[i + 1..] {
                let overlap = ra.intersect(*rb);
                assert!(
                    !overlap.is_positive() || overlap.area() < 1.0,
                    "{a} overlaps {b}"
                );
            }
        }
        no_placeholders(&harness);
    }
}

#[test]
fn an_unmodified_opened_pipeline_has_no_modified_mark() {
    let saved = saved_list(&["Customers per city"]);
    let harness = open(draft().opened_from(&saved[0]), saved, STANDARD);
    assert!(
        harness
            .query_by_label(&t("aggregate.saved.modified"))
            .is_none()
    );
}

#[test]
fn the_open_menu_fits_the_small_window_with_named_items() {
    let names: Vec<String> = (1..=12).map(|i| format!("Pipeline number {i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut harness = open(draft(), saved_list(&refs), SMALL);
    harness
        .get_by_role_and_label(Role::Button, &t("aggregate.saved.open"))
        .click();
    harness.run_steps(3);
    harness.get_by_role_and_label(
        Role::Button,
        &tn("aggregate.saved.rename_n", "Pipeline number 12"),
    );
    harness.get_by_role_and_label(
        Role::Button,
        &tn("aggregate.saved.delete_n", "Pipeline number 12"),
    );
    // The list scrolls inside the menu: the first item (the newest) is inside the window.
    let first_label = tn("aggregate.saved.open_item_n", "Pipeline number 12");
    let first = harness.get_by_role_and_label(Role::Button, &first_label);
    assert!(
        window(SMALL).contains_rect(first.rect()),
        "{:?}",
        first.rect()
    );
    no_placeholders(&harness);
}

/// The dialogs, each with the labels it must show and the buttons it must name.
fn dialogs(saved: &[SavedPipeline]) -> Vec<(SavedDialog, Vec<String>, Vec<String>)> {
    let id = saved[0].id;
    let name = saved[0].name.clone();
    vec![
        (
            SavedDialog::Save {
                name: "Top cities".into(),
                replace_prompt: false,
            },
            vec![t("aggregate.saved.save_title")],
            vec![t("common.cancel"), t("aggregate.saved.save")],
        ),
        (
            SavedDialog::Save {
                name: name.clone(),
                replace_prompt: true,
            },
            vec![
                t("aggregate.saved.save_title"),
                tn("aggregate.saved.replace_message", &name),
            ],
            vec![t("common.cancel"), t("aggregate.saved.replace_confirm")],
        ),
        (
            SavedDialog::ConfirmOpen { id },
            vec![
                t("aggregate.saved.open_title"),
                tn("aggregate.saved.open_message", &name),
            ],
            vec![t("common.cancel"), t("aggregate.saved.open_confirm")],
        ),
        (
            SavedDialog::Rename {
                id,
                name: name.clone(),
            },
            vec![t("aggregate.saved.rename_title")],
            vec![t("common.cancel"), t("aggregate.saved.rename_confirm")],
        ),
        (
            SavedDialog::ConfirmDelete { id },
            vec![
                t("aggregate.saved.delete_title"),
                tn("aggregate.saved.delete_message", &name),
            ],
            vec![t("common.cancel"), t("aggregate.saved.delete_confirm")],
        ),
    ]
}

#[test]
fn every_dialog_is_named_and_fits_both_window_sizes() {
    let saved = saved_list(&["Customers per city"]);
    for size in [STANDARD, SMALL] {
        for (dialog, labels, buttons) in dialogs(&saved) {
            let harness = open(
                draft().with_saved_dialog(Some(dialog.clone())),
                saved.clone(),
                size,
            );
            for label in &labels {
                harness.get_by_label(label);
            }
            for button in &buttons {
                // The header's own Save is still in the tree behind the modal: the dialog's is last.
                let node = harness
                    .get_all_by_role_and_label(Role::Button, button)
                    .last()
                    .expect("the button is there");
                assert!(
                    window(size).contains_rect(node.rect()),
                    "{dialog:?} {button}: {:?}",
                    node.rect()
                );
            }
            no_placeholders(&harness);
        }
    }
}

#[test]
fn a_name_dialog_has_one_named_field_and_tab_walks_field_cancel_confirm() {
    let saved = saved_list(&["Customers per city"]);
    let dialog = SavedDialog::Save {
        name: "Top cities".into(),
        replace_prompt: false,
    };
    let mut harness = open(draft().with_saved_dialog(Some(dialog)), saved, SMALL);
    let field = t("aggregate.saved.name_label");
    harness.get_by_role_and_label(Role::TextInput, &field);
    let mut order = vec![focused(&harness)];
    for _ in 0..2 {
        harness.key_press(egui::Key::Tab);
        harness.run_steps(2);
        order.push(focused(&harness));
    }
    let expected = [field, t("common.cancel"), t("aggregate.saved.save")];
    assert_eq!(order, expected.map(Some));
}

#[test]
fn the_destructive_prompts_start_on_cancel_and_tab_reaches_the_confirm_button() {
    let saved = saved_list(&["Customers per city"]);
    let id = saved[0].id;
    for (dialog, confirm) in [
        (
            SavedDialog::ConfirmDelete { id },
            "aggregate.saved.delete_confirm",
        ),
        (
            SavedDialog::ConfirmOpen { id },
            "aggregate.saved.open_confirm",
        ),
    ] {
        let mut harness = open(
            draft().with_saved_dialog(Some(dialog)),
            saved.clone(),
            SMALL,
        );
        assert_eq!(focused(&harness), Some(t("common.cancel")));
        harness.key_press(egui::Key::Tab);
        harness.run_steps(2);
        assert_eq!(focused(&harness), Some(t(confirm)));
    }
}

#[test]
fn the_dialogs_text_stays_inside_the_small_window_for_the_longest_name() {
    let long = "W".repeat(80);
    let saved = saved_list(&[&long]);
    let id = saved[0].id;
    let harness = open(
        draft().with_saved_dialog(Some(SavedDialog::ConfirmDelete { id })),
        saved,
        SMALL,
    );
    let message = harness.get_by_label_contains(&long);
    assert!(
        window(SMALL).contains_rect(message.rect()),
        "{:?}",
        message.rect()
    );
    assert_laid_out(&harness, SMALL, &[Role::Button]);
}
