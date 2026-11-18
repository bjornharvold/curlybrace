//! Where the index verdict sits on the Browse page: beside the two switches when the window has
//! room for it (1000x700), on the next line when it has not (800x500), and the rows below it
//! never move when a verdict appears. Measured from the accesskit bounds of the REAL app on an
//! in-memory store. Fast, offline, no GPU.
mod support;

use eframe::egui::Rect;
use egui_kittest::kittest::Queryable;
use support::browse_page::{BrowsePage, SIZE_800, SIZE_1000, browse_page, t};
use support::verdict_shots::collection_scan;

fn rect(page: &BrowsePage, key: &str) -> Rect {
    page.harness.get_by_label(&t(key)).rect()
}

#[test]
fn at_1000x700_the_verdict_shares_the_switches_row() {
    let page = browse_page(SIZE_1000, collection_scan);
    let switch = rect(&page, "query.verdict.automatic");
    let why = rect(&page, "query.verdict.why");
    assert!(
        (switch.center().y - why.center().y).abs() <= 2.0,
        "the switch {switch:?} and the verdict's Why button {why:?} are on different rows"
    );
    assert!(
        why.left() > switch.right(),
        "the verdict follows the switches"
    );
}

#[test]
fn at_800x500_the_verdict_starts_a_new_line() {
    let page = browse_page(SIZE_800, collection_scan);
    let switch = rect(&page, "query.verdict.automatic");
    let why = rect(&page, "query.verdict.why");
    assert!(
        why.top() >= switch.bottom() - 1.0,
        "the verdict {why:?} is not under the switch {switch:?}"
    );
}

#[test]
fn nothing_below_the_verdict_moves_when_it_appears() {
    for size in [SIZE_1000, SIZE_800] {
        let with = browse_page(size, collection_scan);
        // No verdict yet: the area is reserved, empty.
        let without = browse_page(size, |_| ());
        for key in ["view.table", "doc.insert", "doc.select_all_result"] {
            let (a, b) = (rect(&with, key), rect(&without, key));
            assert!(
                (a.top() - b.top()).abs() <= 0.5,
                "{size:?} {key}: {a:?} with a verdict, {b:?} without"
            );
        }
    }
}
