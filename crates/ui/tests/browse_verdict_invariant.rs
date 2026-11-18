//! A verdict appearing never moves anything on the Browse page, at any window width: with a page
//! found with options (so the options status sits after the verdict's area) the switches row, the
//! options status, the documents toolbar and the first document row are where they were without
//! a verdict. Measured from the accesskit bounds of the REAL app on an in-memory store. Fast,
//! offline, no GPU.
mod support;

use egui_kittest::kittest::Queryable;
use support::browse_page::{BrowsePage, browse_page, t};
use support::find_options_shots::find_options_page;
use support::verdict_shots::show_collection_scan_verdict;

/// Widths around every wrap point of the header (the reported ones are 760, 770, 910, 1280+).
const WIDTHS: [f32; 16] = [
    700.0, 740.0, 760.0, 770.0, 800.0, 850.0, 900.0, 904.0, 910.0, 912.0, 960.0, 1000.0, 1100.0,
    1200.0, 1280.0, 1400.0,
];
const HEIGHT: f32 = 900.0;
const KEYS: [&str; 4] = [
    "query.verdict.automatic",
    "view.table",
    "doc.insert",
    "doc.select_all_result",
];

fn tops(page: &BrowsePage) -> Vec<(f32, f32)> {
    let mut found: Vec<(f32, f32)> = KEYS
        .iter()
        .map(|key| {
            let rect = page.harness.get_by_label(&t(key)).rect();
            (rect.left(), rect.top())
        })
        .collect();
    found.push((0.0, page.first_row_top()));
    found
}

#[test]
fn a_verdict_moves_nothing_while_the_options_status_is_shown() {
    for width in WIDTHS {
        let with = browse_page((width, HEIGHT), |s| {
            find_options_page(s);
            show_collection_scan_verdict(s);
        });
        let without = browse_page((width, HEIGHT), |s| {
            find_options_page(s);
        });
        for ((a, b), key) in tops(&with)
            .into_iter()
            .zip(tops(&without))
            .zip(KEYS.iter().chain([&"first row"]))
        {
            assert!(
                (a.0 - b.0).abs() <= 0.5 && (a.1 - b.1).abs() <= 0.5,
                "width {width}, {key}: {a:?} with a verdict, {b:?} without"
            );
        }
    }
}
