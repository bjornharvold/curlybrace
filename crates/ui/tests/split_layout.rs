//! The split view's size rules (#159): how narrow and how short a pane of the Browse page can be
//! before its find bar and documents toolbar break the layout audit, measured on the REAL page
//! with a window sized so that the page's area is the pane (the central panel less its margin is
//! what a pane gets). The measured numbers fix `MIN_PANE_WIDTH` and `MIN_PANE_HEIGHT`. Fast,
//! offline, no GPU. Run with `--nocapture` for the measurements.
mod support;

use curlybrace_ui::I18n;
use curlybrace_ui::viewmodel::split_layout::{MIN_PANE_HEIGHT, MIN_PANE_WIDTH, STAIRS};
use eframe::egui::{self, Rect, accesskit::Role};
use egui_kittest::kittest::{NodeT, Queryable};
use support::browse_page::{BrowsePage, browse_page};

/// The central panel's inner margin on every side (egui's `Frame::central_panel`).
const MARGIN: f32 = 8.0;
/// A button, tab or field is at least this tall (points): WCAG 2.2's 24 pt target.
const MIN_TARGET_HEIGHT: f32 = 24.0;
/// Overlaps smaller than this are rounding, not layout (square points).
const OVERLAP_TOLERANCE: f32 = 1.0;
/// How far a measured corner may be off on another platform (points).
const SLACK: f32 = 10.0;
/// The documents a pane must still show for its height to be usable.
const ROWS_WANTED: usize = 3;

/// The Browse page laid out in an area of a known size (the central panel less its margin).
struct Pane {
    page: BrowsePage,
    area: Rect,
}

/// The sidebar's width at the app's default, read from a page of the real app.
fn sidebar_width() -> f32 {
    let page = browse_page((1000.0, 700.0), |_| {});
    egui::containers::panel::PanelState::load(&page.harness.ctx, egui::Id::new("sidebar"))
        .map_or(280.0, |state| state.rect.width())
}

/// The page in a pane of `size`: the window is the pane, the sidebar and the margins.
fn pane(size: egui::Vec2, sidebar: f32) -> Pane {
    let window = (size.x + sidebar + 2.0 * MARGIN, size.y + 2.0 * MARGIN);
    let page = browse_page(window, |_| {});
    let area = Rect::from_min_max(
        egui::pos2(sidebar + MARGIN, MARGIN),
        egui::pos2(window.0 - MARGIN, window.1 - MARGIN),
    );
    Pane { page, area }
}

fn is_control(role: Role) -> bool {
    matches!(
        role,
        Role::Button
            | Role::Tab
            | Role::CheckBox
            | Role::RadioButton
            | Role::ComboBox
            | Role::TextInput
            | Role::MultilineTextInput
    )
}

/// The controls of the page inside the pane's area (the sidebar's are left of it).
fn controls(pane: &Pane) -> Vec<(String, Rect)> {
    pane.page
        .harness
        .root()
        .children_recursive()
        .filter(|n| is_control(n.accesskit_node().role()))
        .map(|n| {
            let name = n.accesskit_node().label().unwrap_or_default();
            (format!("{:?} {name}", n.accesskit_node().role()), n.rect())
        })
        .filter(|(_, rect)| rect.width() > 0.0 && rect.height() > 0.0)
        .filter(|(_, rect)| rect.left() >= pane.area.left() - 1.0)
        .filter(|(_, rect)| rect.top() < pane.area.bottom())
        .collect()
}

/// What breaks the layout audit's rules for the page (no control past the pane's right edge, none
/// squashed below a usable target, none overlapping another), if anything.
fn problems(pane: &Pane) -> Vec<String> {
    let shown = controls(pane);
    let mut found = Vec::new();
    for (label, rect) in &shown {
        if rect.right() > pane.area.right() + 0.5 {
            found.push(format!("{label} runs past the right edge: {rect:?}"));
        }
        let target = ["Button ", "Tab ", "TextInput "]
            .iter()
            .any(|role| label.starts_with(role));
        if target && rect.height() < MIN_TARGET_HEIGHT {
            found.push(format!("{label} is squashed: {rect:?}"));
        }
    }
    for (i, (a, ra)) in shown.iter().enumerate() {
        for (b, rb) in &shown[i + 1..] {
            let overlap = ra.intersect(*rb);
            if overlap.is_positive() && overlap.area() >= OVERLAP_TOLERANCE {
                found.push(format!("{a} {ra:?} overlaps {b} {rb:?}"));
            }
        }
    }
    found
}

/// How many document rows are fully inside the pane.
fn rows_shown(pane: &Pane) -> usize {
    let label = I18n::english().t_with("doc.select_row", &[("n", "")]);
    let prefix = label.trim_end().to_string();
    pane.page
        .harness
        .root()
        .children_recursive()
        .filter(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|name| name.starts_with(&prefix))
        })
        .filter(|n| n.rect().bottom() <= pane.area.bottom() + 0.5)
        .count()
}

/// A pane of `size` that keeps the audit's rules and shows the wanted rows.
fn usable(size: egui::Vec2, sidebar: f32) -> bool {
    let measured = pane(size, sidebar);
    problems(&measured).is_empty() && rows_shown(&measured) >= ROWS_WANTED
}

/// The widths the staircase is measured at (points).
fn widths() -> impl Iterator<Item = f32> {
    (250..=1100).step_by(10).map(|w| w as f32)
}

/// The measured staircase: the corners `(width, height)` of the smallest usable panes. A taller
/// pane may be narrower (its header wraps into more rows), a wider one lower. Each corner is the
/// narrowest width that is usable at its height and the lowest height usable at its width.
fn staircase(sidebar: f32) -> Vec<(f32, f32)> {
    let mut corners: Vec<(f32, f32)> = Vec::new();
    // The lowest height only falls as the width grows: search down from the one before.
    let mut height = 900.0;
    for width in widths() {
        while height > 100.0 && usable(egui::vec2(width, height - 10.0), sidebar) {
            height -= 10.0;
        }
        match corners.last() {
            Some((_, h)) if *h == height => {}
            _ => corners.push((width, height)),
        }
    }
    corners
}

#[test]
fn measure_the_staircase() {
    let sidebar = sidebar_width();
    eprintln!("sidebar {sidebar} wide");
    for (width, height) in staircase(sidebar) {
        eprintln!("corner {width} x {height}");
    }
}

/// Each corner of the table is the measurement: usable, and neither narrower nor lower is.
#[test]
fn pane_minimums_match_the_measured_page() {
    let sidebar = sidebar_width();
    assert!(!STAIRS.is_empty());
    // Each corner is the measurement within one font step either way: a few points of difference
    // between platforms must not fail the test, a table that is out of date must.
    for (width, height) in STAIRS {
        assert!(
            usable(egui::vec2(width + SLACK, height + SLACK), sidebar),
            "{width} x {height} is too small"
        );
        assert!(
            !usable(egui::vec2(width - 2.0 * SLACK, height), sidebar),
            "{width} x {height} could be narrower"
        );
        assert!(
            !usable(egui::vec2(width, height - 2.0 * SLACK), sidebar),
            "{width} x {height} could be lower"
        );
    }
    // The smallest of all, on either axis, are the named constants.
    let narrowest = STAIRS.iter().map(|(w, _)| *w).fold(f32::MAX, f32::min);
    let lowest = STAIRS.iter().map(|(_, h)| *h).fold(f32::MAX, f32::min);
    assert_eq!(MIN_PANE_WIDTH, narrowest);
    assert_eq!(MIN_PANE_HEIGHT, lowest);
}
