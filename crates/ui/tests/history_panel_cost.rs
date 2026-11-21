//! What the History button costs while its popup is closed, and how big a row can get: the lists
//! are built only while the popup is open, the search of the last time is forgotten when it
//! closes, and a row (its text and the name a screen reader gets) is cut at a few hundred
//! characters however long the filter is. Fast, offline, no GPU.
use curlybrace_core::query_history::{FindText, QueryHistory};
use curlybrace_ui::I18n;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::aggregate_hint_list::HintListUi;
use curlybrace_ui::viewmodel::query_history::{HistoryAction, HistoryUi, ROW_CHARS, listing};
use curlybrace_ui::viewmodel::{FindAction, FindBarState};
use curlybrace_ui::views::findbar::show_findbar_history;
use curlybrace_ui::views::query_history::HistoryPanel;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::cell::Cell;
use std::rc::Rc;
use uuid::Uuid;

struct Frame {
    bar: FindBarState,
    history: HistoryUi,
    store: QueryHistory,
    built: Rc<Cell<u32>>,
    actions: Vec<FindAction>,
}

fn open(store: QueryHistory, search: &str) -> (Harness<'static, Frame>, Rc<Cell<u32>>) {
    let built = Rc::new(Cell::new(0));
    let frame = Frame {
        bar: FindBarState::default(),
        history: HistoryUi {
            search: search.into(),
            ..HistoryUi::default()
        },
        store,
        built: built.clone(),
        actions: Vec::new(),
    };
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui_state(
            move |ui, frame: &mut Frame| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                let hints = HintListUi::default().unavailable();
                let (store, search, counter) =
                    (&frame.store, &frame.history.search, frame.built.clone());
                let actions = {
                    let panel = HistoryPanel {
                        ui: &frame.history,
                        listing: Some(Box::new(move || {
                            counter.set(counter.get() + 1);
                            listing(store, (Uuid::nil(), "shop", "orders"), search)
                        }) as Box<dyn Fn() -> _>),
                        now: 1_000_000,
                        remembering: true,
                    };
                    show_findbar_history(
                        ui,
                        &mut frame.bar,
                        (false, false),
                        (&hints, Some(&panel)),
                        &I18n::english(),
                    )
                };
                for action in &actions {
                    if let FindAction::History(HistoryAction::Search(s)) = action {
                        frame.history.search = s.clone();
                    }
                }
                frame.actions.extend(actions);
            },
            frame,
        );
    harness.run_steps(4);
    (harness, built)
}

fn store_with(filter: &str) -> QueryHistory {
    QueryHistory::default()
        .recorded(
            Uuid::nil(),
            "shop",
            "orders",
            FindText {
                filter: filter.into(),
                ..FindText::default()
            },
            1,
        )
        .unwrap()
}

#[test]
fn a_closed_popup_builds_no_lists() {
    let (mut harness, built) = open(store_with("{}"), "");
    harness.run_steps(5);
    assert_eq!(built.get(), 0, "nothing was built for a closed popup");
    let label = I18n::english().t("history.button");
    harness.get_by_role_and_label(Role::Button, &label).click();
    harness.run_steps(4);
    assert!(built.get() > 0, "the open popup built them");
}

#[test]
fn the_search_of_the_last_opening_is_forgotten_when_the_popup_is_closed() {
    let (mut harness, _) = open(store_with("{}"), "old search");
    harness.run_steps(3);
    assert_eq!(harness.state().history.search, "", "reset while closed");
}

#[test]
fn a_row_for_a_huge_filter_is_cut_in_its_text_and_in_its_name() {
    let filter = format!("{{ \"a\": \"{}\" }}", "x".repeat(200_000));
    let (mut harness, _) = open(store_with(&filter), "");
    let label = I18n::english().t("history.button");
    harness.get_by_role_and_label(Role::Button, &label).click();
    harness.run_steps(4);
    let load = I18n::english().t_with("history.row.load", &[("text", "")]);
    let prefix = load.trim_end();
    let longest = harness
        .root()
        .children_recursive()
        .filter_map(|n| n.accesskit_node().label())
        .filter(|l| l.starts_with(prefix))
        .map(|l| l.chars().count())
        .max()
        .expect("the row is there");
    assert!(
        longest <= prefix.chars().count() + ROW_CHARS + 8,
        "the row's name has {longest} characters"
    );
}
