//! The More row of the find bar on the Browse page: Sort, Projection, Skip, Limit and the Query
//! options toggle sit on aligned rows (one row at 1000x700, two at 800x500) and never run past the
//! panel's right edge. Measured from the accesskit bounds of the REAL app on an in-memory store.
//! Fast, offline, no GPU.
mod support;

use curlybrace_ui::viewmodel::FindBarState;
use curlybrace_ui::viewmodel::aggregate_options::OptionsEdit;
use eframe::egui::Rect;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use support::browse_page::{BrowsePage, SIZE_800, SIZE_1000, bar, browse_page, t};

/// Two options set, so the toggle reads "Query options (2)" and is wider than without a count.
fn with_two_options(state: &mut curlybrace_ui::app::state::AppState) {
    bar(true, Some(false))(state);
    state.find = [
        OptionsEdit::MaxTime("5000".into()),
        OptionsEdit::Comment("why".into()),
    ]
    .into_iter()
    .fold(state.find.clone(), FindBarState::options_edited);
}

struct Row {
    sort: Rect,
    projection: Rect,
    skip: Rect,
    limit: Rect,
    toggle: Rect,
    filter: Rect,
}

fn row(page: &BrowsePage, toggle_label: &str) -> Row {
    let harness = &page.harness;
    let field = |key: &str| {
        harness
            .get_by_role_and_label(Role::TextInput, &t(key))
            .rect()
    };
    Row {
        sort: field("findbar.sort"),
        projection: field("findbar.projection"),
        skip: field("findbar.skip"),
        limit: field("findbar.limit"),
        toggle: harness
            .get_by_role_and_label(Role::Button, toggle_label)
            .rect(),
        filter: field("findbar.filter"),
    }
}

fn aligned(rects: &[Rect], what: &str) {
    let first = rects[0].top();
    for rect in rects {
        assert!(
            (rect.top() - first).abs() <= 1.0,
            "{what}: tops differ: {rects:?}"
        );
    }
}

#[test]
fn at_1000x700_the_more_row_is_one_aligned_row() {
    let page = browse_page(SIZE_1000, bar(true, Some(false)));
    let r = row(&page, &t("query.options.toggle"));
    aligned(
        &[r.sort, r.projection, r.skip, r.limit, r.toggle],
        "no options set",
    );
    assert!(r.sort.right() <= r.projection.left() && r.projection.right() <= r.skip.left());
    assert!(r.limit.right() <= r.toggle.left());
}

#[test]
fn at_1000x700_the_row_stays_one_row_with_options_set() {
    let page = browse_page(SIZE_1000, with_two_options);
    let label = curlybrace_ui::I18n::english().t_with("query.options.toggle_count", &[("n", "2")]);
    let r = row(&page, &label);
    aligned(
        &[r.sort, r.projection, r.skip, r.limit, r.toggle],
        "two options set",
    );
    assert!(
        r.sort.width() >= 159.0 && r.projection.width() >= 159.0,
        "Sort {:?} and Projection {:?} keep their minimum",
        r.sort,
        r.projection
    );
}

#[test]
fn at_800x500_the_row_wraps_to_two_aligned_rows_inside_the_panel() {
    let page = browse_page(SIZE_800, bar(true, Some(false)));
    let r = row(&page, &t("query.options.toggle"));
    aligned(&[r.sort, r.projection], "first row");
    aligned(&[r.skip, r.limit, r.toggle], "second row");
    assert!(
        r.skip.top() > r.sort.bottom(),
        "the second row is under the first"
    );
    // On a row of its own the Filter spans the panel, so its right edge is the panel's.
    let edge = r.filter.right() + 0.5;
    for rect in [r.sort, r.projection, r.skip, r.limit, r.toggle] {
        assert!(
            rect.right() <= edge,
            "{rect:?} runs past the panel edge {edge}"
        );
    }
}
