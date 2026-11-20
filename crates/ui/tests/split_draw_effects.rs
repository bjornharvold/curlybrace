//! What drawing the other pane does to the state (#159, slice 3 review): a stale partner that
//! becomes visible reads again (a window growing from one pane to two), and a pane that refreshes
//! itself on its own does not take the focus. Fast, offline, no GPU.
mod support;

use curlybrace_ui::app::state::AppState;
use curlybrace_ui::app::tabs::Freshness;
use curlybrace_ui::viewmodel::CollectionTab;
use curlybrace_ui::viewmodel::split_layout::Orientation;
use eframe::egui;
use egui_kittest::kittest::{NodeT, Queryable};
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

/// The real app in a harness whose closure also runs `after(ctx)` after each frame of the app.
fn page_with(
    size: (f32, f32),
    setup: impl FnOnce(&mut AppState),
    mut after: impl FnMut(&egui::Context) + 'static,
) -> (
    egui_kittest::Harness<'static, curlybrace_ui::app::CurlybraceApp>,
    tokio::runtime::Runtime,
    tempfile::TempDir,
) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("tokio runtime");
    let enter = rt.enter();
    let (mut app, dir) = support::shots::build_app(vec![], None);
    setup(app.state_mut());
    let mut installed = false;
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .with_pixels_per_point(1.0)
        .build_state(
            move |ctx, app: &mut curlybrace_ui::app::CurlybraceApp| {
                if !installed {
                    curlybrace_ui::theme::install(ctx);
                    installed = true;
                }
                app.run_frame(ctx);
                after(ctx);
            },
            app,
        );
    harness.run_steps(6);
    drop(enter);
    (harness, rt, dir)
}

#[test]
fn an_overlay_that_has_closed_does_not_stop_a_press_activating_the_other_pane() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let open = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&open);
    let (mut harness, _rt, _dir) = page_with(
        (1000.0, 700.0),
        |s| split_two(s, Orientation::Stacked),
        move |ctx| {
            if flag.load(Ordering::SeqCst) {
                egui::Area::new(egui::Id::new("gone_overlay"))
                    .order(egui::Order::Foreground)
                    .interactable(false)
                    .fixed_pos(egui::pos2(500.0, 520.0))
                    .show(ctx, |ui| {
                        ui.label("a tooltip that is about to close, and long enough to cover the middle of the pane");
                    });
            }
        },
    );
    // The overlay closes; its layer is still remembered by egui, with its last rectangle.
    open.store(false, Ordering::SeqCst);
    harness.run_steps(4);
    let split = harness.state().state().tabs.split.clone().expect("a split");
    let other = harness
        .query_all_by_role(egui::accesskit::Role::Group)
        .find(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Other pane"))
        })
        .expect("the other pane");
    other.click();
    harness.run_steps(4);
    assert_eq!(
        harness.state().state().active_tab(),
        Some(split.secondary),
        "a press where an overlay used to be activates the pane"
    );
}

#[test]
fn an_empty_stale_partner_does_not_reset_the_limit_notice() {
    let page = browse_page((1400.0, 900.0), |s| {
        split_two(s, Orientation::SideBySide);
        let partner = s.tabs.split.as_ref().expect("a split").secondary;
        if let Some(view) = s.parked_tabs.get_mut(&partner) {
            *view = Default::default();
            view.freshness = Freshness::Stale;
        }
        s.tabs = std::mem::take(&mut s.tabs)
            .with_notice(Some(curlybrace_ui::app::tab_strip::TabNotice::LimitReached));
    });
    // browse_page ran 6 frames already: the notice outlives them.
    assert!(
        state(&page).tabs.notice.is_some(),
        "the automatic pane step must not dismiss the notice"
    );
}

#[test]
fn f6_is_not_consumed_without_a_split() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let left = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&left);
    let (mut harness, _rt, _dir) = page_with(
        (1000.0, 700.0),
        |s| {
            support::tabs_shots::browsing_with_tabs(s, &["invoices"]);
        },
        move |ctx| {
            let there = ctx.input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::F6,
                            pressed: true,
                            ..
                        }
                    )
                })
            });
            if there {
                seen.store(true, Ordering::SeqCst);
            }
        },
    );
    harness.key_press(egui::Key::F6);
    harness.run_steps(3);
    assert!(
        left.load(Ordering::SeqCst),
        "without a split F6 is left for others"
    );
}

#[test]
fn a_stale_one_pane_flag_does_not_leak_into_a_new_split() {
    // One pane at 800x500 (the flag is set), the split ends, the window grows, a new split starts:
    // its first frame shows the orientation button.
    let mut page = browse_page((800.0, 500.0), |s| split_two(s, Orientation::SideBySide));
    page.harness
        .state_mut()
        .dispatch(curlybrace_ui::app::reducer::Input::Tabs(
            curlybrace_ui::app::tab_actions::TabAction::ToggleSplit,
        ));
    page.harness.set_size(egui::vec2(1400.0, 900.0));
    page.harness.run_steps(3);
    page.harness
        .state_mut()
        .dispatch(curlybrace_ui::app::reducer::Input::Tabs(
            curlybrace_ui::app::tab_actions::TabAction::ToggleSplit,
        ));
    // One pass only: the first pass of the first frame is what shows the stale flag.
    page.harness
        .ctx
        .options_mut(|o| o.max_passes = std::num::NonZeroUsize::new(1).expect("one"));
    page.harness.step();
    assert!(
        page.harness
            .query_all_by_label("Panes stacked")
            .next()
            .is_some(),
        "the first frame of the new split shows the button"
    );
}

#[test]
fn an_empty_stale_partner_emits_no_pane_shown() {
    // Nothing to read: the pane does not ask for a read, however many frames are drawn.
    let mut page = browse_page((1400.0, 900.0), |s| {
        split_two(s, Orientation::SideBySide);
        let partner = s.tabs.split.as_ref().expect("a split").secondary;
        if let Some(view) = s.parked_tabs.get_mut(&partner) {
            *view = Default::default();
            view.freshness = Freshness::Stale;
        }
    });
    page.harness.run_steps(6);
    assert_eq!(
        curlybrace_ui::views::pane_scope::pane_shown_emitted(&page.harness.ctx),
        0
    );
}
