//! The collection tab strip in the accesskit tree and under the keyboard (#159): a tab list of
//! named tabs, every control named, opening, switching and closing by keyboard and mouse, the
//! limit notice and the dormant tab's Run. Fast, offline, no GPU.
mod support;

use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::app::tab_actions::MAX_TABS;
use eframe::egui::{self, Key, Modifiers, PointerButton, accesskit::Role};
use egui_kittest::kittest::{NodeT, Queryable};
use support::browse_page::{BrowsePage, SIZE_1000, browse_page};
use support::tabs_shots::{browsing_with_tabs, make_dormant, parked_tab};

/// The English text of `key`, kept for the whole test run: a query borrows its label.
fn t(key: &str) -> &'static str {
    Box::leak(support::browse_page::t(key).into_boxed_str())
}

fn page_with_tabs(others: &[&'static str]) -> BrowsePage {
    browse_page(SIZE_1000, move |state| {
        browsing_with_tabs(state, others);
    })
}

fn state(page: &BrowsePage) -> &AppState {
    page.harness.state().state()
}

fn app(page: &mut BrowsePage) -> &mut CurlybraceApp {
    page.harness.state_mut()
}

fn press(page: &mut BrowsePage, mods: Modifiers, key: Key) {
    page.harness.key_press_modifiers(mods, key);
    page.harness.run_steps(3);
}

#[test]
fn strip_is_a_tab_list_with_named_tabs() {
    let page = page_with_tabs(&["invoices", "notes"]);
    let list = page
        .harness
        .get_by_role_and_label(Role::TabList, t("tabs.strip"));
    let tabs: Vec<_> = page.harness.query_all_by_role(Role::Tab).collect();
    assert_eq!(tabs.len(), 3, "one tab per collection");
    for tab in &tabs {
        let name = tab.accesskit_node().label().unwrap_or_default();
        assert!(name.contains("of 3"), "{name}");
        assert!(name.contains("Production"), "names the connection: {name}");
        assert!(tab.rect().top() >= list.rect().top() - 1.0);
    }
    let first = tabs[0].accesskit_node().label().unwrap_or_default();
    assert!(
        first.contains("customers") && first.contains("tab 1 of 3"),
        "{first}"
    );
}

#[test]
fn the_tabs_are_children_of_the_tab_list_in_the_tree() {
    let page = page_with_tabs(&["invoices", "notes"]);
    let tabs: Vec<_> = page.harness.query_all_by_role(Role::Tab).collect();
    assert_eq!(tabs.len(), 3);
    for tab in &tabs {
        let parent = tab.accesskit_node().parent().map(|p| p.role());
        assert_eq!(parent, Some(Role::TabList), "the tree, not the rects");
    }
}

#[test]
fn all_tabs_menu_entries_of_duplicates_are_told_apart() {
    // A second tab on the same collection, put together without a read: a duplicate's find would
    // go to a runtime that never connected the fixture's session and fail it under load.
    let mut page = browse_page(SIZE_1000, |state| {
        browsing_with_tabs(state, &[]);
        let session = state.active.expect("session");
        parked_tab(state, session, "crm", "customers");
    });
    page.harness
        .get_by_role_and_label(Role::Button, t("tabs.all_tabs"))
        .click();
    page.harness.run_steps(6);
    let names: Vec<String> = page
        .harness
        .query_all_by_label_contains("customers, tab")
        .map(|n| n.accesskit_node().label().unwrap_or_default())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert_ne!(names[0], names[1]);
}

#[test]
fn a_dormant_tab_says_it_needs_run_and_is_drawn_dimmer() {
    let page = browse_page(SIZE_1000, |state| {
        let tabs = browsing_with_tabs(state, &["invoices"]);
        make_dormant(state, tabs[0]);
    });
    let names: Vec<String> = page
        .harness
        .query_all_by_role(Role::Tab)
        .map(|n| n.accesskit_node().label().unwrap_or_default())
        .collect();
    let needs = t("tabs.state.needs_run");
    assert!(names.iter().any(|n| n.contains(needs)), "{names:?}");
    assert!(
        names.iter().any(|n| !n.contains(needs)),
        "only the dormant one says it: {names:?}"
    );
}

#[test]
fn a_tab_with_both_badges_keeps_room_for_its_title() {
    let page = browse_page(SIZE_1000, |state| {
        browsing_with_tabs(state, &[]);
        support::tabs_shots::background_session(state, "Billing", "transactions", (true, true));
    });
    let tabs: Vec<_> = page.harness.query_all_by_role(Role::Tab).collect();
    let both = tabs
        .iter()
        .find(|n| {
            n.accesskit_node()
                .label()
                .unwrap_or_default()
                .contains("transactions")
        })
        .expect("the transactions tab");
    assert!(both.rect().width() >= 70.0, "96 less the close button");
    // The marks are compact: the title keeps at least 60 pt.
    let title = page
        .harness
        .query_all_by_role_and_label(Role::Label, "transactions")
        .next()
        .expect("the title");
    assert!(title.rect().width() >= 60.0, "{:?}", title.rect());
    let name = both.accesskit_node().label().unwrap_or_default();
    assert!(name.contains(t("tabs.state.production")), "{name}");
    assert!(name.contains(t("tabs.state.read_only")), "{name}");
}

#[test]
fn a_single_tab_shows_no_strip() {
    let page = page_with_tabs(&[]);
    assert!(
        page.harness
            .query_by_role_and_label(Role::TabList, t("tabs.strip"))
            .is_none(),
        "someone who never opens a second tab sees no change"
    );
}

#[test]
fn open_switch_close_by_keyboard() {
    let mut page = page_with_tabs(&["invoices", "notes"]);
    let order = state(&page).tabs.order.clone();
    // Cmd+T opens an empty tab after the one shown; it is the active one and has a strip.
    press(&mut page, Modifiers::COMMAND, Key::T);
    assert_eq!(state(&page).tabs.order.len(), 4);
    assert_eq!(state(&page).selected_ns(), None);
    page.harness
        .get_by_role_and_label(Role::TabList, t("tabs.strip"));
    // Cmd+1 shows the first tab again: the empty tab nobody used is not kept.
    press(&mut page, Modifiers::COMMAND, Key::Num1);
    assert_eq!(state(&page).active_tab(), Some(order[0]));
    assert_eq!(state(&page).tabs.order.len(), 3);
    // Cmd+Shift+] and Cmd+Shift+[ walk the strip; Cmd+9 is the last tab.
    let next = Modifiers::COMMAND | Modifiers::SHIFT;
    press(&mut page, next, Key::CloseBracket);
    assert_eq!(state(&page).active_tab(), Some(order[1]));
    press(&mut page, next, Key::OpenBracket);
    assert_eq!(state(&page).active_tab(), Some(order[0]));
    press(&mut page, Modifiers::COMMAND, Key::Num9);
    assert_eq!(state(&page).active_tab(), Some(order[2]));
    // Cmd+W closes it and Cmd+Shift+T brings it back (as a new tab, with its collection).
    press(&mut page, Modifiers::COMMAND, Key::W);
    assert_eq!(state(&page).tabs.order.len(), 2);
    press(&mut page, next, Key::T);
    assert_eq!(state(&page).tabs.order.len(), 3);
    assert_eq!(
        state(&page).selected_ns().map(|ns| ns.collection.as_str()),
        Some("notes")
    );
}

#[test]
fn ctrl_tab_walks_the_tabs_most_recently_used_first() {
    let mut page = page_with_tabs(&["invoices"]);
    let first = state(&page).active_tab().expect("active");
    press(&mut page, Modifiers::CTRL, Key::Tab);
    let second = state(&page).active_tab().expect("active");
    assert_ne!(first, second);
    press(&mut page, Modifiers::CTRL, Key::Tab);
    assert_eq!(state(&page).active_tab(), Some(first), "back and forth");
}

#[test]
fn middle_click_opens_in_new_tab() {
    let mut page = page_with_tabs(&[]);
    let before = state(&page).tabs.order.len();
    let row = page.harness.get_by_role_and_label(Role::Button, "invoices");
    row.click_button(PointerButton::Middle);
    page.harness.step();
    assert_eq!(state(&page).tabs.order.len(), before + 1);
    assert_eq!(
        state(&page).selected_ns().map(|ns| ns.collection.as_str()),
        Some("invoices")
    );
}

#[test]
fn context_menu_open_in_new_tab() {
    let mut page = page_with_tabs(&[]);
    page.harness
        .get_by_role_and_label(Role::Button, "notes")
        .click_secondary();
    page.harness.run_steps(3);
    page.harness
        .get_by_role_and_label(Role::Button, t("tabs.open_in_new_tab"))
        .click();
    page.harness.step();
    assert_eq!(state(&page).tabs.order.len(), 2);
    assert_eq!(
        state(&page).selected_ns().map(|ns| ns.collection.as_str()),
        Some("notes")
    );
}

#[test]
fn alt_enter_on_a_focused_collection_opens_in_new_tab() {
    let mut page = page_with_tabs(&[]);
    page.harness
        .get_by_role_and_label(Role::Button, "invoices")
        .focus();
    page.harness.run_steps(2);
    press(&mut page, Modifiers::ALT, Key::Enter);
    assert_eq!(state(&page).tabs.order.len(), 2);
    assert_eq!(
        state(&page).selected_ns().map(|ns| ns.collection.as_str()),
        Some("invoices")
    );
}

#[test]
fn focus_order_sidebar_strip_header_content() {
    let page = page_with_tabs(&["invoices"]);
    let sidebar = page
        .harness
        .get_by_role_and_label(Role::Button, t("sidebar.refresh"));
    let tab = page
        .harness
        .query_all_by_role(Role::Tab)
        .next()
        .expect("a tab");
    let header = page
        .harness
        .get_by_role_and_label(Role::Button, t("browser.tab_documents"));
    let content = page
        .harness
        .get_by_role_and_label(Role::Button, t("findbar.run"));
    // The sidebar is to the left; then the strip above the header above the content.
    assert!(sidebar.rect().right() <= tab.rect().left() + 1.0);
    assert!(tab.rect().bottom() <= header.rect().top() + 1.0);
    assert!(header.rect().bottom() <= content.rect().top() + 1.0);
}

#[test]
fn close_button_is_named_after_its_tab() {
    let mut page = page_with_tabs(&["invoices"]);
    let close = page
        .harness
        .get_by_role_and_label(Role::Button, "Close invoices");
    close.click();
    page.harness.run_steps(3);
    assert_eq!(state(&page).tabs.order.len(), 1);
    assert!(
        page.harness
            .query_by_role_and_label(Role::Button, "Close invoices")
            .is_none()
    );
}

#[test]
fn all_tabs_menu_lists_every_tab() {
    let mut page = page_with_tabs(&["invoices", "notes"]);
    page.harness
        .get_by_role_and_label(Role::Button, t("tabs.all_tabs"))
        .click();
    page.harness.run_steps(3);
    for title in ["customers", "invoices", "notes"] {
        let entries = page.harness.query_all_by_label(title).count();
        assert!(entries >= 1, "the menu lists {title}");
    }
    let before = state(&page).active_tab();
    // Choosing an entry shows that tab.
    let notes: Vec<_> = page.harness.query_all_by_label("notes").collect();
    notes.last().expect("an entry").click();
    page.harness.run_steps(3);
    assert_ne!(state(&page).active_tab(), before);
}

#[test]
fn limit_notice_is_announced() {
    let mut page = browse_page(SIZE_1000, |state| {
        browsing_with_tabs(state, &[]);
        let session = state.active.expect("session");
        for i in 0..(MAX_TABS - 1) {
            parked_tab(state, session, "crm", &format!("c{i}"));
        }
    });
    page.harness
        .get_by_role_and_label(Role::Button, t("tabs.new_tab"))
        .click();
    page.harness.run_steps(3);
    let text = egui_text("20 tabs are open. Close one to open another.");
    let notice = page.harness.get_by_label(&text);
    assert_eq!(
        notice.accesskit_node().live(),
        egui::accesskit::Live::Polite,
        "a live region, so a screen reader says it"
    );
}

fn egui_text(s: &str) -> String {
    s.to_string()
}

#[test]
fn dormant_tab_offers_run() {
    let mut page = browse_page(SIZE_1000, |state| {
        let tabs = browsing_with_tabs(state, &["invoices"]);
        // The shown tab lost its connection: no page, waiting for Run.
        state.page = None;
        state.page_key = None;
        let current = state.active_tab().expect("tab");
        make_dormant(state, current);
        let _ = tabs;
    });
    let run = page
        .harness
        .get_by_role_and_label(Role::Button, t("tabs.dormant_run"));
    run.click();
    page.harness.step();
    assert!(
        state(&page).loading_page,
        "its Run sent the find: {:?}",
        state(&page).issued_request
    );
    let _ = app(&mut page);
}
