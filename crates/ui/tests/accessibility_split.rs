//! The split view in the accesskit tree and under the keyboard (#159, slice 3): a named divider
//! that reports its value and moves with the arrow keys, F6 and Shift+F6 between the panes, focus
//! that follows a click, every pane named by its connection and badge, the one-pane chip, and no
//! egui ID clash in a split frame. Fast, offline, no GPU.
mod support;

use curlybrace_ui::app::state::AppState;
use curlybrace_ui::app::tab_actions::TabAction;
use curlybrace_ui::viewmodel::FindAction;
use curlybrace_ui::viewmodel::split_layout::Orientation;
use eframe::egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::kittest::{NodeT, Queryable};
use support::browse_page::{BrowsePage, SIZE_800, browse_page};
use support::tabs_shots::{split_two, split_two_connections};

/// Wide and tall enough for two panes side by side.
const WIDE: (f32, f32) = (1400.0, 900.0);
/// The standard window: the panes are stacked when asked to be.
const STANDARD: (f32, f32) = (1000.0, 700.0);

fn t(key: &str) -> &'static str {
    Box::leak(support::browse_page::t(key).into_boxed_str())
}

fn split_page(size: (f32, f32)) -> BrowsePage {
    browse_page(size, |state| split_two(state, Orientation::SideBySide))
}

/// The group of the pane whose name starts with `prefix` ("Active pane", "Other pane").
fn pane_group<'a>(page: &'a BrowsePage, prefix: &str) -> egui_kittest::Node<'a> {
    page.harness
        .query_all_by_role(Role::Group)
        .find(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with(prefix))
        })
        .unwrap_or_else(|| panic!("no pane group named {prefix}"))
}

fn state(page: &BrowsePage) -> &AppState {
    page.harness.state().state()
}

fn press(page: &mut BrowsePage, mods: Modifiers, key: Key) {
    page.harness.key_press_modifiers(mods, key);
    page.harness.run_steps(4);
}

fn ratio(page: &BrowsePage) -> f32 {
    state(page).tabs.split.as_ref().expect("a split").ratio
}

#[test]
fn divider_is_named_and_reports_its_value() {
    let page = split_page(WIDE);
    let divider = page
        .harness
        .get_by_role_and_label(Role::Slider, t("tabs.split.divider"));
    let node = divider.accesskit_node();
    assert_eq!(node.numeric_value(), Some(50.0));
    let (lo, hi) = (
        node.min_numeric_value().expect("min"),
        node.max_numeric_value().expect("max"),
    );
    assert!(
        lo > 25.0 && hi < 75.0 && lo + hi == 100.0,
        "the limits are what the other pane leaves: {lo} {hi}"
    );
    assert_eq!(node.value().as_deref(), Some("50%"));
    // The bar is between the panes and as long as they are tall.
    let rect = divider.rect();
    assert_eq!(rect.width(), 8.0, "{rect:?}");
    assert!(rect.height() > 300.0, "{rect:?}");
}

#[test]
fn arrow_keys_move_the_divider_home_resets() {
    let mut page = split_page(WIDE);
    page.harness
        .get_by_role_and_label(Role::Slider, t("tabs.split.divider"))
        .focus();
    page.harness.run_steps(3);
    press(&mut page, Modifiers::NONE, Key::ArrowRight);
    assert_eq!(ratio(&page), 0.55, "one step right");
    press(&mut page, Modifiers::NONE, Key::ArrowRight);
    assert_eq!(ratio(&page), 0.6);
    press(&mut page, Modifiers::NONE, Key::ArrowLeft);
    assert_eq!(ratio(&page), 0.55);
    // Focus stayed on the divider: egui did not use the arrows to move it.
    let divider = page
        .harness
        .get_by_role_and_label(Role::Slider, t("tabs.split.divider"));
    assert!(divider.accesskit_node().is_focused());
    let value = divider.accesskit_node().value().map(|v| v.to_string());
    assert_eq!(value.as_deref(), Some("55%"));
    press(&mut page, Modifiers::NONE, Key::Home);
    assert_eq!(ratio(&page), 0.5, "Home puts it back in the middle");
    // It never leaves a quarter to three quarters.
    // At the pane the other side needs (about 63% here) the divider stops: the slider reports
    // where it is, which is the same after another press.
    let percent = |page: &BrowsePage| {
        page.harness
            .get_by_role_and_label(Role::Slider, t("tabs.split.divider"))
            .accesskit_node()
            .numeric_value()
    };
    for _ in 0..8 {
        press(&mut page, Modifiers::NONE, Key::ArrowRight);
    }
    let stopped = percent(&page);
    assert!(
        stopped.is_some_and(|p| (55.0..=75.0).contains(&p)),
        "{stopped:?}"
    );
    press(&mut page, Modifiers::NONE, Key::ArrowRight);
    assert_eq!(percent(&page), stopped);
    // At either stop the value is inside the limits the slider reports (one rounding for all three).
    let divider = |page: &BrowsePage| {
        let node = page
            .harness
            .get_by_role_and_label(Role::Slider, t("tabs.split.divider"));
        let n = node.accesskit_node();
        (
            n.numeric_value().expect("value"),
            n.min_numeric_value().expect("min"),
            n.max_numeric_value().expect("max"),
        )
    };
    let (value, lo, hi) = divider(&page);
    assert!(
        (lo..=hi).contains(&value),
        "at the right stop: {lo} <= {value} <= {hi}"
    );
    for _ in 0..30 {
        press(&mut page, Modifiers::NONE, Key::ArrowLeft);
    }
    let (value, lo, hi) = divider(&page);
    assert!(
        (lo..=hi).contains(&value),
        "at the left stop: {lo} <= {value} <= {hi}"
    );
    let max = page
        .harness
        .get_by_role_and_label(Role::Slider, t("tabs.split.divider"))
        .accesskit_node()
        .max_numeric_value()
        .expect("max");
    assert!(
        f64::from(ratio(&page)) * 100.0 <= max + 1.0,
        "the stored ratio never passes the visible stop"
    );
}

#[test]
fn f6_moves_focus_to_the_other_pane() {
    let mut page = split_page(WIDE);
    let split = state(&page).tabs.split.clone().expect("a split");
    assert_eq!(state(&page).active_tab(), Some(split.primary));
    press(&mut page, Modifiers::NONE, Key::F6);
    assert_eq!(state(&page).active_tab(), Some(split.secondary), "F6");
    let now = state(&page).tabs.split.clone().expect("still split");
    assert_eq!(
        (now.primary, now.secondary),
        (split.secondary, split.primary)
    );
    // The keyboard is on the pane that is now the active one.
    let active = pane_group(&page, "Active pane");
    assert!(
        active.accesskit_node().is_focused(),
        "the new active pane has the keyboard"
    );
    press(&mut page, Modifiers::SHIFT, Key::F6);
    assert_eq!(
        state(&page).active_tab(),
        Some(split.primary),
        "Shift+F6 goes back"
    );
}

#[test]
fn f6_works_in_a_text_field_but_not_behind_a_dialog() {
    let mut page = split_page(WIDE);
    let first = state(&page).active_tab();
    // The first of the two panes' Filter fields: the active pane's.
    page.harness
        .query_all_by_role_and_label(Role::TextInput, t("findbar.filter"))
        .next()
        .expect("a filter field")
        .click();
    page.harness.run_steps(3);
    press(&mut page, Modifiers::NONE, Key::F6);
    assert_ne!(state(&page).active_tab(), first, "F6 is not a text key");
    // Behind a dialog nothing moves.
    let mut page = split_page(WIDE);
    page.harness
        .state_mut()
        .dispatch(curlybrace_ui::app::reducer::Input::Find(
            FindAction::CopyAsCode,
        ));
    page.harness.run_steps(4);
    let before = state(&page).active_tab();
    press(&mut page, Modifiers::NONE, Key::F6);
    assert_eq!(state(&page).active_tab(), before, "a dialog holds F6 back");
}

#[test]
fn cmd_backslash_splits_and_unsplits_but_not_while_typing() {
    let mut page = browse_page(WIDE, |state| {
        support::tabs_shots::browsing_with_tabs(state, &["invoices"]);
    });
    assert!(state(&page).tabs.split.is_none());
    press(&mut page, Modifiers::COMMAND, Key::Backslash);
    assert!(state(&page).tabs.split.is_some(), "Cmd+\\ splits");
    press(&mut page, Modifiers::COMMAND, Key::Backslash);
    assert!(state(&page).tabs.split.is_none(), "and again ends it");
    // Not with an extra modifier, not while typing.
    press(
        &mut page,
        Modifiers::COMMAND | Modifiers::SHIFT,
        Key::Backslash,
    );
    assert!(
        state(&page).tabs.split.is_none(),
        "Cmd+Shift+\\ is another chord"
    );
    page.harness
        .get_by_role_and_label(Role::TextInput, t("findbar.filter"))
        .click();
    page.harness.run_steps(3);
    press(&mut page, Modifiers::COMMAND, Key::Backslash);
    assert!(state(&page).tabs.split.is_none(), "typing is never taken");
}

#[test]
fn clicking_the_other_pane_makes_it_active() {
    let mut page = split_page(WIDE);
    let split = state(&page).tabs.split.clone().expect("a split");
    pane_group(&page, "Other pane").click();
    page.harness.run_steps(4);
    assert_eq!(state(&page).active_tab(), Some(split.secondary));
    let now = state(&page).tabs.split.clone().expect("still split");
    assert_eq!(now.primary, split.secondary);
    assert_eq!(now.secondary, split.primary);
    assert!(now.secondary_first, "nothing moved on screen");
}

#[test]
fn each_pane_header_names_its_connection_and_badge() {
    let page = browse_page(WIDE, |state| {
        split_two_connections(state, Orientation::SideBySide);
    });
    let name = |which: &str| {
        pane_group(&page, which)
            .accesskit_node()
            .label()
            .unwrap_or_default()
    };
    let (active, other) = (name("Active pane"), name("Other pane"));
    assert!(active.contains("Production"), "{active}");
    assert!(
        other.contains("Staging"),
        "names its own connection: {other}"
    );
    assert!(
        other.contains("production connection"),
        "and its badge: {other}"
    );
    // Each pane's header draws its own connection.
    assert!(page.harness.query_all_by_label("Staging").count() >= 1);
}

#[test]
fn single_pane_chip_and_show_other_pane() {
    let mut page = browse_page(SIZE_800, |state| split_two(state, Orientation::SideBySide));
    let chip = t("tabs.split.single_show_other");
    // One pane: no divider, no group per pane, the chip says why.
    assert_eq!(page.harness.query_all_by_role(Role::Slider).count(), 0);
    // Turning the panes would change nothing here: the button is not drawn (it is at WIDE).
    let turn = || {
        page.harness
            .query_all_by_role_and_label(Role::Button, t("tabs.split.orientation_stacked"))
            .count()
    };
    assert_eq!(turn(), 0, "no orientation button with one pane");
    assert_eq!(
        split_page(WIDE)
            .harness
            .query_all_by_role_and_label(Role::Button, t("tabs.split.orientation_stacked"))
            .count(),
        1
    );
    page.harness
        .get_by_label_contains("Split view needs a larger window");
    let split = state(&page).tabs.split.clone().expect("a split");
    page.harness
        .get_by_role_and_label(Role::Button, chip)
        .click();
    page.harness.run_steps(4);
    assert_eq!(state(&page).active_tab(), Some(split.secondary));
}

#[test]
fn a_stacked_split_has_a_horizontal_divider() {
    let page = browse_page(STANDARD, |state| split_two(state, Orientation::Stacked));
    let divider = page
        .harness
        .get_by_role_and_label(Role::Slider, t("tabs.split.divider"));
    let rect = divider.rect();
    assert_eq!(rect.height(), 8.0, "{rect:?}");
    assert!(rect.width() > 400.0, "{rect:?}");
}

#[test]
fn the_strip_closes_the_split_and_turns_the_panes() {
    let mut page = split_page(WIDE);
    page.harness
        .get_by_role_and_label(Role::Button, t("tabs.split.orientation_stacked"))
        .click();
    page.harness.run_steps(4);
    let now = state(&page).tabs.split.clone().expect("still split");
    assert_eq!(now.orientation, Orientation::Stacked);
    page.harness
        .get_by_role_and_label(Role::Button, t("tabs.split.off"))
        .click();
    page.harness.run_steps(4);
    assert!(state(&page).tabs.split.is_none());
    let _ = TabAction::ToggleSplit;
}

#[test]
fn a_pending_f6_focus_does_not_grab_the_focus_after_a_resize() {
    // One pane fits at 800x500: F6 swaps the roles, and no pane frame is drawn to take the focus.
    let mut page = browse_page(SIZE_800, |state| split_two(state, Orientation::SideBySide));
    press(&mut page, Modifiers::NONE, Key::F6);
    page.harness.set_size(eframe::egui::vec2(1400.0, 900.0));
    page.harness.run_steps(6);
    let focused = page
        .harness
        .root()
        .children_recursive()
        .filter(|n| n.accesskit_node().role() == Role::Group)
        .any(|n| n.accesskit_node().is_focused());
    assert!(
        !focused,
        "a stale F6 target took the keyboard after the resize"
    );
}

#[test]
fn focus_order_is_sidebar_strip_active_pane_divider_other_pane() {
    let page = split_page(WIDE);
    // The order keyboard focus visits the controls is the order of the accessibility tree.
    let order: Vec<(Role, String)> = page
        .harness
        .root()
        .accesskit_node()
        .children()
        .map(|n| (n.role(), n.label().unwrap_or_default()))
        .collect();
    let at = |want: &dyn Fn(&(Role, String)) -> bool| {
        order
            .iter()
            .position(want)
            .unwrap_or_else(|| panic!("not in the tree: {order:?}"))
    };
    let sidebar = at(&|(_, l)| l == t("tools.settings_button"));
    let strip = at(&|(r, _)| *r == Role::TabList);
    let active = at(&|(r, l)| *r == Role::Group && l.starts_with("Active pane"));
    let divider = at(&|(r, _)| *r == Role::Slider);
    let other = at(&|(r, l)| *r == Role::Group && l.starts_with("Other pane"));
    assert!(
        sidebar < strip && strip < active && active < divider && divider < other,
        "{sidebar} {strip} {active} {divider} {other}"
    );
}

#[test]
fn the_strip_is_drawn_when_the_partner_is_an_empty_pane() {
    let page = browse_page(WIDE, |state| {
        split_two(state, Orientation::SideBySide);
        let partner = state.tabs.split.as_ref().expect("a split").secondary;
        if let Some(view) = state.parked_tabs.get_mut(&partner) {
            *view = Default::default();
        }
    });
    page.harness
        .get_by_role_and_label(Role::TabList, t("tabs.strip"));
    // Both panes' tabs are in it, the empty one under the empty-tab title, and the buttons are.
    assert_eq!(page.harness.query_all_by_role(Role::Tab).count(), 2);
    page.harness
        .get_by_role_and_label(Role::Button, t("tabs.split.off"));
}
