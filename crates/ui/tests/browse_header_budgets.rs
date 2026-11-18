//! How high the Browse page's header lets the first document row start, at the two window sizes
//! the app is designed for, with More closed, More open, and More and the Query options section
//! open. Measured from the accesskit bounds of the REAL app on an in-memory store. Fast, offline,
//! no GPU. The numbers are `docs/superpowers/specs/2026-11-18-browse-header-design.md`'s budgets.
mod support;

use support::browse_page::{
    CLOSED_800, CLOSED_1000, MORE_800, MORE_1000, MORE_AND_OPTIONS_1000, SIZE_800, SIZE_1000, bar,
    browse_page,
};

fn assert_within(
    size: (f32, f32),
    find: impl FnOnce(&mut curlybrace_ui::app::state::AppState),
    budget: f32,
) {
    let page = browse_page(size, find);
    let top = page.first_row_top();
    eprintln!("{size:?}: the first document starts at y = {top} (budget {budget})");
    assert!(
        top <= budget,
        "{size:?}: the first document starts at y = {top} (budget {budget})"
    );
}

#[test]
fn more_closed_1000x700() {
    assert_within(SIZE_1000, bar(false, None), CLOSED_1000);
}

#[test]
fn more_open_1000x700() {
    assert_within(SIZE_1000, bar(true, Some(false)), MORE_1000);
}

#[test]
fn more_and_query_options_open_1000x700() {
    assert_within(SIZE_1000, bar(true, Some(true)), MORE_AND_OPTIONS_1000);
}

#[test]
fn more_closed_800x500() {
    assert_within(SIZE_800, bar(false, None), CLOSED_800);
}

#[test]
fn more_open_800x500() {
    assert_within(SIZE_800, bar(true, Some(false)), MORE_800);
}
