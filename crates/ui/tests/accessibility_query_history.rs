//! The History button, popup and dialogs in the accesskit tree, rendered with the REAL English
//! locale at the standard 1000x700 and the small 800x500 window: the button is in the find bar's
//! button row without crowding it, the popup's controls have roles and names and stay inside the
//! window, rows load and star by click and by keyboard, the search announces its match count, and
//! every dialog is named, fits and starts on the safe control. Fast, offline, no GPU.
use curlybrace_core::query_history::{FindText, QueryHistory};
use curlybrace_ui::I18n;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::aggregate_hint_list::HintListUi;
use curlybrace_ui::viewmodel::query_history::{Dialog, HistoryAction, HistoryUi, listing};
use curlybrace_ui::viewmodel::{FindAction, FindBarState};
use curlybrace_ui::views::findbar::show_findbar_history;
use curlybrace_ui::views::query_history::HistoryPanel;
use curlybrace_ui::views::query_history_dialogs::show_history_dialog;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use uuid::Uuid;

const STANDARD: (f32, f32) = (1000.0, 700.0);
const SMALL: (f32, f32) = (800.0, 500.0);
const NOW: i64 = 1_000_000;
const CONN: Uuid = Uuid::nil();

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn tn(key: &str, params: &[(&str, &str)]) -> String {
    I18n::english().t_with(key, params)
}

fn text(filter: &str) -> FindText {
    FindText {
        filter: filter.into(),
        ..FindText::default()
    }
}

/// Three recent queries (the newest first) and two favourites, all for `shop.orders`.
fn sample() -> QueryHistory {
    QueryHistory::default()
        .recorded(
            CONN,
            "shop",
            "orders",
            text("{ \"status\": \"open\" }"),
            NOW - 600,
        )
        .unwrap()
        .recorded(
            CONN,
            "shop",
            "orders",
            text("{ \"total\": { \"$gt\": 5 } }"),
            NOW - 7200,
        )
        .unwrap()
        .recorded(
            CONN,
            "shop",
            "orders",
            text("{ \"city\": \"Oslo\" }"),
            NOW - 10,
        )
        .unwrap()
        .starred(
            CONN,
            "shop",
            "orders",
            "Open orders",
            text("{ \"status\": \"open\" }"),
            1,
        )
        .unwrap()
        .starred(
            CONN,
            "shop",
            "orders",
            "Big baskets",
            text("{ \"total\": { \"$gt\": 100 } }"),
            2,
        )
        .unwrap()
}

struct Frame {
    bar: FindBarState,
    history: HistoryUi,
    store: QueryHistory,
    collection: bool,
    remembering: bool,
    actions: Vec<FindAction>,
}

fn frame(store: QueryHistory) -> Frame {
    Frame {
        bar: FindBarState::default(),
        history: HistoryUi::default(),
        store,
        collection: true,
        remembering: true,
        actions: Vec::new(),
    }
}

fn open(state: Frame, size: (f32, f32)) -> Harness<'static, Frame> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .build_ui_state(
            move |ui, frame: &mut Frame| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                let hints = HintListUi::default().unavailable();
                let panel = HistoryPanel {
                    ui: &frame.history,
                    listing: frame.collection.then(|| {
                        listing(
                            &frame.store,
                            (CONN, "shop", "orders"),
                            &frame.history.search,
                        )
                    }),
                    now: NOW,
                    remembering: frame.remembering,
                };
                let actions = show_findbar_history(
                    ui,
                    &mut frame.bar,
                    (false, false),
                    (&hints, Some(&panel)),
                    &I18n::english(),
                );
                for action in &actions {
                    if let FindAction::History(HistoryAction::Search(search)) = action {
                        frame.history.search = search.clone();
                    }
                }
                frame.actions.extend(actions);
            },
            state,
        );
    harness.run_steps(4);
    harness
}

fn opened(state: Frame, size: (f32, f32)) -> Harness<'static, Frame> {
    let mut harness = open(state, size);
    harness
        .get_by_role_and_label(Role::Button, &t("history.button"))
        .click();
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

fn no_placeholders(harness: &Harness<'static, Frame>) {
    for node in harness.root().children_recursive() {
        let label = node.accesskit_node().label().unwrap_or_default();
        assert!(!label.contains('\u{27e6}'), "{label}");
    }
}

fn history_actions(harness: &Harness<'static, Frame>) -> Vec<HistoryAction> {
    harness
        .state()
        .actions
        .iter()
        .filter_map(|a| match a {
            FindAction::History(h) => Some(h.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_button_sits_in_the_button_row_named_and_inside_the_window() {
    for size in [STANDARD, SMALL] {
        let harness = open(frame(sample()), size);
        let (button, run_label, clear_label) =
            (t("history.button"), t("findbar.run"), t("findbar.clear"));
        let history = harness.get_by_role_and_label(Role::Button, &button);
        assert!(window(size).contains_rect(history.rect()), "{size:?}");
        let find = harness.get_by_role_and_label(Role::Button, &run_label);
        let clear = harness.get_by_role_and_label(Role::Button, &clear_label);
        // After Find and Clear, no overlap with any button of the row.
        assert!(history.rect().min.x >= clear.rect().max.x - 1.0);
        for other in [
            t("findbar.run"),
            t("findbar.clear"),
            t("findbar.prev"),
            t("findbar.next"),
        ] {
            let node = harness.get_by_role_and_label(Role::Button, &other);
            let overlap = history.rect().intersect(node.rect());
            assert!(
                !overlap.is_positive() || overlap.area() < 1.0,
                "{other} overlaps History at {size:?}"
            );
        }
        let _ = find;
    }
}

#[test]
fn without_a_collection_the_button_is_off_and_says_why() {
    let mut state = frame(sample());
    state.collection = false;
    let mut harness = open(state, STANDARD);
    let label = t("history.button");
    let button = harness.get_by_role_and_label(Role::Button, &label);
    assert!(button.accesskit_node().is_disabled());
    assert_eq!(
        button.accesskit_node().description().as_deref(),
        Some(t("history.button_no_collection").as_str())
    );
    button.click();
    harness.run_steps(3);
    assert!(
        harness.query_by_label(&t("history.search")).is_none(),
        "no popup opens"
    );
}

#[test]
fn the_popup_names_its_controls_and_fits_both_window_sizes() {
    for size in [STANDARD, SMALL] {
        let harness = opened(frame(sample()), size);
        harness.get_by_role_and_label(Role::TextInput, &t("history.search"));
        harness.get_by_label(&t("history.section.favourites"));
        harness.get_by_label(&t("history.section.recent"));
        harness.get_by_label(&t("history.privacy"));
        harness.get_by_role_and_label(Role::Button, &t("history.clear"));
        let row = tn("history.row.load", &[("text", "{ \"city\": \"Oslo\" }")]);
        let node = harness.get_by_role_and_label(Role::Button, &row);
        assert!(window(size).contains_rect(node.rect()), "{size:?}");
        harness.get_by_role_and_label(
            Role::Button,
            &tn("history.star_n", &[("text", "{ \"city\": \"Oslo\" }")]),
        );
        let fav = tn(
            "history.row.load_favourite",
            &[
                ("name", "Open orders"),
                ("text", "{ \"status\": \"open\" }"),
            ],
        );
        harness.get_by_role_and_label(Role::Button, &fav);
        harness.get_by_role_and_label(
            Role::Button,
            &tn("history.rename_n", &[("name", "Open orders")]),
        );
        harness.get_by_role_and_label(
            Role::Button,
            &tn("history.delete_n", &[("name", "Open orders")]),
        );
        harness.get_by_label("10 min ago");
        no_placeholders(&harness);
    }
}

#[test]
fn a_row_loads_by_click_and_by_enter_with_its_place_in_the_list() {
    // In the order they were recorded, newest first: Oslo (0), total (1), open (2). A row below the popup's fold is reached by
    // keyboard; one in view by click.
    let mut harness = opened(frame(sample()), STANDARD);
    let favourite = tn(
        "history.row.load_favourite",
        &[
            ("name", "Big baskets"),
            ("text", "{ \"total\": { \"$gt\": 100 } }"),
        ],
    );
    harness
        .get_by_role_and_label(Role::Button, &favourite)
        .click();
    harness.run_steps(3);
    assert!(matches!(
        history_actions(&harness).as_slice(),
        [HistoryAction::LoadFavourite(_)]
    ));

    let mut harness = opened(frame(sample()), STANDARD);
    let row = tn("history.row.load", &[("text", "{ \"status\": \"open\" }")]);
    harness.get_by_role_and_label(Role::Button, &row).focus();
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(history_actions(&harness), [HistoryAction::LoadRecent(2)]);
}

#[test]
fn clicking_star_or_a_favourites_buttons_asks_for_them() {
    let mut harness = opened(frame(sample()), STANDARD);
    harness
        .get_by_role_and_label(
            Role::Button,
            &tn("history.star_n", &[("text", "{ \"city\": \"Oslo\" }")]),
        )
        .click();
    harness.run_steps(3);
    assert_eq!(history_actions(&harness), [HistoryAction::StarAsked(0)]);

    let mut harness = opened(frame(sample()), STANDARD);
    harness
        .get_by_role_and_label(
            Role::Button,
            &tn("history.rename_n", &[("name", "Big baskets")]),
        )
        .click();
    harness.run_steps(3);
    assert!(matches!(
        history_actions(&harness).as_slice(),
        [HistoryAction::RenameAsked(_)]
    ));
}

#[test]
fn typing_in_the_search_keeps_the_popup_open_and_announces_the_match_count() {
    let mut harness = opened(frame(sample()), STANDARD);
    let search = t("history.search");
    let field = harness.get_by_role_and_label(Role::TextInput, &search);
    field.click();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, &t("history.search"))
        .type_text("oslo");
    harness.run_steps(4);
    assert_eq!(harness.state().history.search, "oslo");
    // The popup is still open and the list followed the search.
    harness.get_by_role_and_label(Role::TextInput, &t("history.search"));
    let one = t("history.matches_one");
    let count = harness.get_by_label(&one);
    assert_eq!(
        count.accesskit_node().live(),
        egui::accesskit::Live::Polite,
        "the count is announced"
    );
    assert!(
        harness
            .query_by_role_and_label(
                Role::Button,
                &tn("history.row.load", &[("text", "{ \"status\": \"open\" }")])
            )
            .is_none()
    );
}

#[test]
fn a_search_with_no_match_says_so() {
    let mut state = frame(sample());
    state.history.search = "zzz".into();
    let harness = opened(state, STANDARD);
    harness.get_by_label(&t("history.matches_none"));
}

#[test]
fn empty_states_are_plain_sentences() {
    let harness = opened(frame(QueryHistory::default()), SMALL);
    harness.get_by_label(&t("history.empty.favourites"));
    harness.get_by_label(&t("history.empty.recent"));
    assert!(
        harness
            .query_by_role_and_label(Role::Button, &t("history.clear"))
            .is_none(),
        "nothing to clear"
    );
    let mut off = frame(QueryHistory::default());
    off.remembering = false;
    let harness = opened(off, SMALL);
    harness.get_by_label(&t("history.empty.recent_off"));
}

#[test]
fn clear_history_asks_to_clear() {
    let mut harness = opened(frame(sample()), STANDARD);
    harness
        .get_by_role_and_label(Role::Button, &t("history.clear"))
        .click();
    harness.run_steps(3);
    assert_eq!(history_actions(&harness), [HistoryAction::ClearAsked]);
}

#[test]
fn tab_walks_the_popup_in_visual_order() {
    // Opened from the keyboard: the button has the focus and Enter opens the popup.
    let mut harness = open(frame(sample()), STANDARD);
    let label = t("history.button");
    harness.get_by_role_and_label(Role::Button, &label).focus();
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(4);
    let mut order = vec![focused(&harness)];
    for _ in 0..6 {
        harness.key_press(egui::Key::Tab);
        harness.run_steps(2);
        order.push(focused(&harness));
    }
    let labels: Vec<String> = order.into_iter().flatten().collect();
    // The button, then the search field, then the favourites (newest saved first) each with its
    // row buttons, before any recent row.
    let position = |needle: &str| labels.iter().position(|l| l.contains(needle));
    let search = position(&t("history.search")).unwrap_or_else(|| panic!("{labels:?}"));
    let favourite = position("Big baskets").unwrap_or_else(|| panic!("{labels:?}"));
    assert_eq!(labels[0], label);
    assert!(search < favourite, "{labels:?}");
    assert!(
        position("Load query").is_none_or(|recent| favourite < recent),
        "favourites are reached before recent rows: {labels:?}"
    );
}

#[test]
fn arrow_keys_move_between_rows_and_enter_loads() {
    let mut harness = opened(frame(sample()), STANDARD);
    harness
        .get_by_role_and_label(
            Role::Button,
            &tn("history.row.load", &[("text", "{ \"city\": \"Oslo\" }")]),
        )
        .focus();
    harness.run_steps(2);
    harness.key_press(egui::Key::ArrowDown);
    harness.run_steps(3);
    let now = focused(&harness).unwrap_or_default();
    assert_ne!(
        now,
        tn("history.row.load", &[("text", "{ \"city\": \"Oslo\" }")]),
        "Down left the first row: {now}"
    );
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert!(
        history_actions(&harness).iter().any(|a| matches!(
            a,
            HistoryAction::LoadRecent(_) | HistoryAction::StarAsked(_)
        )),
        "Enter activated the focused control: {:?}",
        harness.state().actions
    );
}

#[test]
fn escape_closes_the_popup() {
    let mut harness = opened(frame(sample()), STANDARD);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(4);
    assert!(harness.query_by_label(&t("history.search")).is_none());
}

struct Dlg {
    ui: HistoryUi,
    store: QueryHistory,
    actions: Vec<HistoryAction>,
}

fn dialog(
    make: impl FnOnce(&QueryHistory) -> HistoryUi,
    size: (f32, f32),
) -> Harness<'static, Dlg> {
    let store = sample();
    let ui = make(&store);
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .build_state(
            move |ctx, dlg: &mut Dlg| {
                if !installed {
                    theme::install(ctx);
                    installed = true;
                }
                let target = (CONN, "shop", "orders");
                let shown =
                    show_history_dialog(ctx, &dlg.ui, (&dlg.store, target), &I18n::english());
                dlg.actions.extend(shown);
            },
            Dlg {
                ui,
                store,
                actions: Vec::new(),
            },
        );
    harness.run_steps(4);
    harness
}

fn focused_in<S>(harness: &Harness<'static, S>) -> Option<String> {
    harness
        .root()
        .children_recursive()
        .find(|n| n.accesskit_node().is_focused())
        .and_then(|n| n.accesskit_node().label())
}

fn favourite(store: &QueryHistory) -> (Uuid, String) {
    let first = &store.favourites(CONN)[0];
    (first.id, first.name.clone())
}

fn showing(d: Dialog) -> impl FnOnce(&QueryHistory) -> HistoryUi {
    move |_| HistoryUi {
        dialog: Some(d),
        ..HistoryUi::default()
    }
}

#[test]
fn every_dialog_is_named_and_fits_both_window_sizes() {
    let (id, name) = favourite(&sample());
    let cases = [
        (
            Dialog::Star {
                text: Box::new(text("{}")),
                name: "Fresh".into(),
            },
            vec![t("history.dialog.star_title")],
            vec![t("common.cancel"), t("history.dialog.star_confirm")],
        ),
        (
            Dialog::Rename {
                id,
                name: name.clone(),
            },
            vec![t("history.dialog.rename_title")],
            vec![t("common.cancel"), t("history.dialog.rename_confirm")],
        ),
        (
            Dialog::Delete { id },
            vec![
                t("history.dialog.delete_title"),
                tn("history.dialog.delete_message", &[("name", &name)]),
            ],
            vec![t("common.cancel"), t("history.dialog.delete_confirm")],
        ),
        (
            Dialog::Clear,
            vec![
                t("history.dialog.clear_title"),
                t("history.dialog.clear_message"),
            ],
            vec![t("common.cancel"), t("history.dialog.clear_confirm")],
        ),
    ];
    for size in [STANDARD, SMALL] {
        for (d, labels, buttons) in &cases {
            let harness = dialog(
                |store| {
                    // The ids of this harness's own store, not of the sample above.
                    let (id, _) = favourite(store);
                    showing(match d {
                        Dialog::Rename { name, .. } => Dialog::Rename {
                            id,
                            name: name.clone(),
                        },
                        Dialog::Delete { .. } => Dialog::Delete { id },
                        other => other.clone(),
                    })(store)
                },
                size,
            );
            for label in labels {
                harness.get_by_label(label);
            }
            for button in buttons {
                let node = harness.get_by_role_and_label(Role::Button, button);
                assert!(window(size).contains_rect(node.rect()), "{d:?} {button}");
            }
            for node in harness.root().children_recursive() {
                let label = node.accesskit_node().label().unwrap_or_default();
                assert!(!label.contains('\u{27e6}'), "{label}");
            }
        }
    }
}

#[test]
fn a_name_dialog_starts_in_its_field_and_tab_walks_field_cancel_confirm() {
    let d = Dialog::Star {
        text: Box::new(text("{}")),
        name: "Fresh".into(),
    };
    let mut harness = dialog(showing(d), SMALL);
    let field = t("history.dialog.name_label");
    harness.get_by_role_and_label(Role::TextInput, &field);
    let mut order = vec![focused_in(&harness)];
    for _ in 0..2 {
        harness.key_press(egui::Key::Tab);
        harness.run_steps(2);
        order.push(focused_in(&harness));
    }
    let expected = [field, t("common.cancel"), t("history.dialog.star_confirm")];
    assert_eq!(order, expected.map(Some));
}

#[test]
fn a_taken_name_shows_its_message_and_switches_confirm_off() {
    let d = Dialog::Star {
        text: Box::new(text("{}")),
        name: "open ORDERS".into(),
    };
    let harness = dialog(showing(d), SMALL);
    let field_label = t("history.dialog.name_label");
    let field = harness.get_by_role_and_label(Role::TextInput, &field_label);
    assert_eq!(
        field.accesskit_node().description(),
        Some(t("history.error.name_taken")),
        "the message is the field's description"
    );
    let confirm_label = t("history.dialog.star_confirm");
    let confirm = harness.get_by_role_and_label(Role::Button, &confirm_label);
    assert!(confirm.accesskit_node().is_disabled());
}

#[test]
fn the_destructive_prompts_start_on_cancel() {
    for delete in [true, false] {
        let mut harness = dialog(
            move |store| {
                let (id, _) = favourite(store);
                let d = if delete {
                    Dialog::Delete { id }
                } else {
                    Dialog::Clear
                };
                showing(d)(store)
            },
            SMALL,
        );
        assert_eq!(focused_in(&harness), Some(t("common.cancel")));
        harness.key_press(egui::Key::Enter);
        harness.run_steps(3);
        assert_eq!(
            harness.state().actions,
            [HistoryAction::DialogCancelled],
            "a stray Enter changes nothing"
        );
    }
}

#[test]
fn a_failed_confirm_is_shown_inside_the_dialog() {
    let harness = dialog(
        |_| HistoryUi {
            dialog: Some(Dialog::Clear),
            error: Some("history.error.unusable"),
            ..HistoryUi::default()
        },
        SMALL,
    );
    harness.get_by_label(&t("history.error.unusable"));
}

#[test]
fn a_dialog_for_a_favourite_that_is_gone_cancels_itself() {
    let harness = dialog(
        showing(Dialog::Delete {
            id: Uuid::from_u128(77),
        }),
        SMALL,
    );
    let actions = &harness.state().actions;
    assert!(!actions.is_empty());
    assert!(actions.iter().all(|a| *a == HistoryAction::DialogCancelled));
}
