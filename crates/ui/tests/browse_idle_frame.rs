//! An idle frame changes nothing on the Browse page: after the page has settled, another frame
//! leaves every node where it was, so the header's measured widths are settled in one frame and
//! no layout flickers between two states. Measured from the accesskit bounds of the REAL app on
//! an in-memory store. Fast, offline, no GPU.
mod support;

use curlybrace_ui::app::state::AppState;
use egui_kittest::kittest::NodeT;
use support::browse_page::{BrowsePage, SIZE_800, SIZE_1000, bar, browse_page};
use support::verdict_shots::show_collection_scan_verdict;

/// Every node's role, name and bounds, in tree order.
fn layout(page: &BrowsePage) -> Vec<String> {
    page.harness
        .root()
        .children_recursive()
        .map(|node| {
            let ak = node.accesskit_node();
            format!("{:?} {:?} {:?}", ak.role(), ak.label(), node.rect())
        })
        .collect()
}

fn assert_idle(size: (f32, f32), adjust: impl FnOnce(&mut AppState), what: &str) {
    let mut page = browse_page(size, adjust);
    let settled = layout(&page);
    assert!(!settled.is_empty(), "{what}: no nodes");
    for frame in 1..=2 {
        page.harness.run_steps(1);
        assert_eq!(
            settled,
            layout(&page),
            "{what} at {size:?}: frame {frame} after settling changed the layout"
        );
    }
}

fn verdict_and_more(state: &mut AppState) {
    bar(true, Some(false))(state);
    show_collection_scan_verdict(state);
}

fn verdict_more_and_options(state: &mut AppState) {
    bar(true, Some(true))(state);
    show_collection_scan_verdict(state);
}

#[test]
fn an_idle_frame_changes_nothing_with_a_verdict() {
    for size in [SIZE_1000, SIZE_800] {
        assert_idle(size, show_collection_scan_verdict, "verdict");
    }
}

#[test]
fn an_idle_frame_changes_nothing_with_more_open() {
    for size in [SIZE_1000, SIZE_800] {
        assert_idle(size, verdict_and_more, "verdict and More");
    }
}

#[test]
fn an_idle_frame_changes_nothing_with_more_and_query_options_open() {
    for size in [SIZE_1000, SIZE_800] {
        assert_idle(size, verdict_more_and_options, "verdict, More and options");
    }
}
