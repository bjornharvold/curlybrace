//! The documents toolbar of the Browse page is at most two rows at the two window sizes the app is
//! designed for, and "Select all" says in its hover text and accessible description what the
//! short label leaves out. Measured from the accesskit bounds of the REAL app on an in-memory
//! store. Fast, offline, no GPU.
mod support;

use curlybrace_ui::I18n;
use egui_kittest::kittest::{NodeT, Queryable};
use support::browse_page::{SIZE_800, SIZE_1000, browse_page, t};

/// Every control of the toolbar, in reading order.
const TOOLBAR: [&str; 8] = [
    "view.tree",
    "view.json",
    "doc.insert",
    "bulk.update_matching",
    "bulk.delete_matching",
    "doc.select_all_result",
    "doc.clear_selection",
    "copy.menu.copy",
];

/// How many rows the toolbar's controls fall into (centres within 2 points are one row).
fn toolbar_rows(size: (f32, f32)) -> usize {
    let page = browse_page(size, |_| ());
    let mut centres: Vec<f32> = TOOLBAR
        .iter()
        .map(|key| page.harness.get_by_label(&t(key)).rect().center().y)
        .collect();
    centres.sort_by(f32::total_cmp);
    centres
        .windows(2)
        .filter(|pair| pair[1] - pair[0] > 2.0)
        .count()
        + 1
}

#[test]
fn the_toolbar_is_at_most_two_rows_at_1000x700() {
    let rows = toolbar_rows(SIZE_1000);
    assert!(rows <= 2, "the toolbar takes {rows} rows at 1000x700");
}

#[test]
fn the_toolbar_is_at_most_two_rows_at_800x500() {
    let rows = toolbar_rows(SIZE_800);
    assert!(rows <= 2, "the toolbar takes {rows} rows at 800x500");
}

#[test]
fn select_all_is_short_and_carries_the_full_meaning_as_description_and_hover_text() {
    let mut page = browse_page(SIZE_1000, |_| ());
    let i18n = I18n::english();
    let label = i18n.t("doc.select_all_result");
    let hint = i18n.t("doc.select_all_result_hint");
    let button = page.harness.get_by_label(&label);
    assert_eq!(label, "Select all");
    assert_eq!(
        button.accesskit_node().description().as_deref(),
        Some(hint.as_str())
    );
    button.hover();
    page.harness.run_steps(30);
    page.harness.get_by_label(&hint);
}
