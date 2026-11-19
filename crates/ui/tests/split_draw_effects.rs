//! What drawing the other pane does to the state (#159, slice 3 review): a stale partner that
//! becomes visible reads again (a window growing from one pane to two), and a pane that refreshes
//! itself on its own does not take the focus. Fast, offline, no GPU.
mod support;

use curlybrace_ui::app::state::AppState;
use curlybrace_ui::app::tabs::Freshness;
use curlybrace_ui::viewmodel::CollectionTab;
use curlybrace_ui::viewmodel::split_layout::Orientation;
use eframe::egui;
use egui_kittest::kittest::Queryable;
use support::browse_page::{BrowsePage, browse_page};
use support::tabs_shots::split_two;

fn state(page: &BrowsePage) -> &AppState {
    page.harness.state().state()
}

#[test]
fn a_stale_partner_reads_again_when_a_resize_shows_both_panes() {
    // One pane at 800x500: the partner is not on screen. It has lost its page (evicted).
    let mut page = browse_page((800.0, 500.0), |s| {
        split_two(s, Orientation::SideBySide);
        let partner = s.tabs.split.as_ref().expect("a split").secondary;
        if let Some(view) = s.parked_tabs.get_mut(&partner) {
            view.page = None;
            view.page_key = None;
            view.freshness = Freshness::Stale;
        }
    });
    let partner = state(&page).tabs.split.as_ref().expect("a split").secondary;
    assert_eq!(
        state(&page).freshness_of(partner),
        Freshness::Stale,
        "hidden: not read"
    );
    // The window grows: both panes are drawn and the partner is read.
    page.harness.set_size(egui::vec2(1400.0, 900.0));
    page.harness.run_steps(6);
    assert_eq!(
        state(&page).freshness_of(partner),
        Freshness::Fresh,
        "the pane that became visible read what it shows again"
    );
}

#[test]
fn the_other_panes_automatic_refresh_does_not_take_the_focus() {
    // The other pane shows an idle Aggregate draft: on first show it asks for its preview itself.
    let mut page = browse_page((1400.0, 900.0), |s| {
        split_two(s, Orientation::SideBySide);
        let partner = s.tabs.split.as_ref().expect("a split").secondary;
        if let Some(view) = s.parked_tabs.get_mut(&partner) {
            view.tab = CollectionTab::Aggregate;
            view.aggregate = Box::new(view.aggregate.as_ref().clone().add_stage("$match"));
        }
    });
    page.harness.run_steps(6);
    let split = state(&page).tabs.split.clone().expect("a split");
    let partner = state(&page)
        .parked_tabs
        .get(&split.secondary)
        .expect("the partner is parked");
    assert!(
        format!("{:?}", partner.aggregate.preview) != "Idle",
        "the other pane asked for its own preview"
    );
    assert_eq!(
        state(&page).active_tab(),
        Some(split.primary),
        "and the active tab did not change"
    );
}

#[test]
fn the_orientation_button_is_there_exactly_when_two_panes_are() {
    // Across the threshold between two stacked panes and one: the strip and the panes decide on
    // the same area, so the button never shows with one pane or hides with two.
    let turn = "Panes side by side";
    for height in (560..=720).step_by(6) {
        let page = browse_page((1000.0, height as f32), |s| {
            split_two(s, Orientation::Stacked)
        });
        let two = page
            .harness
            .query_all_by_role(eframe::egui::accesskit::Role::Slider)
            .count()
            == 1;
        let button = page.harness.query_all_by_label(turn).next().is_some();
        assert_eq!(
            button, two,
            "height {height}: button {button}, two panes {two}"
        );
    }
}
