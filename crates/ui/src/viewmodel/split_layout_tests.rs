use super::{
    DIVIDER, MIN_PANE_HEIGHT, MIN_PANE_WIDTH, Orientation, PANE_RING, STAIRS, SplitLayout, clamped,
    layout, min_height_at, min_width_at, ratio_limits,
};
use eframe::egui::vec2;

/// The central area of a window of `w` x `h` with the 280 pt sidebar, the 8 pt margins and the
/// tab strip's row (28 pt and the 4 pt gap) above the panes.
fn central(w: f32, h: f32) -> eframe::egui::Vec2 {
    vec2(w - 280.0 - 16.0, h - 16.0 - 32.0)
}

#[test]
fn side_by_side_when_wide() {
    let got = layout(central(1400.0, 900.0), Orientation::SideBySide, 0.5);
    assert_eq!(got, SplitLayout::Side { ratio: 0.5 });
}

#[test]
fn stacked_at_1000x700() {
    let got = layout(central(1000.0, 700.0), Orientation::Stacked, 0.5);
    assert_eq!(got, SplitLayout::Stacked { ratio: 0.5 });
}

#[test]
fn stacked_is_used_at_1000x700_even_when_side_by_side_is_asked_for() {
    // Two 348 pt wide panes are narrower than the header needs (400); two 322 pt high ones fit.
    let got = layout(central(1000.0, 700.0), Orientation::SideBySide, 0.5);
    assert_eq!(got, SplitLayout::Stacked { ratio: 0.5 });
}

#[test]
fn single_at_800x500() {
    for preferred in [Orientation::SideBySide, Orientation::Stacked] {
        let got = layout(central(800.0, 500.0), preferred, 0.5);
        assert_eq!(got, SplitLayout::Single, "{preferred:?}");
    }
}

#[test]
fn preferred_stacked_is_kept_when_it_fits() {
    let wide = layout(central(1700.0, 900.0), Orientation::Stacked, 0.4);
    assert_eq!(wide, SplitLayout::Stacked { ratio: 0.4 });
    // Stacked asked for, no height for it: the side by side layout that does fit is used.
    let short = vec2(1104.0, 400.0);
    assert_eq!(
        layout(short, Orientation::Stacked, 0.5),
        SplitLayout::Side { ratio: 0.5 }
    );
}

#[test]
fn ratio_is_clamped() {
    assert_eq!(clamped(0.0), 0.25);
    assert_eq!(clamped(1.0), 0.75);
    assert_eq!(clamped(f32::NAN), 0.5);
    // Wide enough that the quarter-to-three-quarters limit is the one that binds (a pane at 0.75
    // of 2400 leaves the other 600, more than the header needs).
    let got = layout(vec2(2408.0, 900.0), Orientation::SideBySide, 0.99);
    assert_eq!(got, SplitLayout::Side { ratio: 0.75 });
    // Narrower, the other pane's minimum binds first.
    let got = layout(central(1700.0, 900.0), Orientation::SideBySide, 0.99);
    assert!(
        matches!(got, SplitLayout::Side { ratio } if ratio < 0.75),
        "{got:?}"
    );
}

#[test]
fn the_ratio_never_squeezes_a_pane_below_its_minimum() {
    // Just wide enough for two panes of the narrowest width that works at this height (470 at
    // 400 high, and the 2 pt ring of each pane on each side): the divider cannot move at all.
    let narrow = vec2(
        2.0 * (470.0 + 2.0 * PANE_RING) + DIVIDER,
        400.0 + 2.0 * PANE_RING,
    );
    assert_eq!(
        layout(narrow, Orientation::SideBySide, 0.25),
        SplitLayout::Side { ratio: 0.5 }
    );
}

#[test]
fn a_narrower_pane_needs_a_taller_one() {
    assert_eq!(min_width_at(600.0), Some(400.0));
    assert_eq!(min_width_at(400.0), Some(470.0));
    assert_eq!(min_width_at(200.0), None);
    assert_eq!(min_height_at(399.0), None);
    assert_eq!(min_height_at(400.0), Some(430.0));
    assert_eq!(min_height_at(1000.0), Some(250.0));
    // 446 wide panes fit when they are 600 high, not when they are 400 high.
    let tall = layout(vec2(900.0, 600.0), Orientation::SideBySide, 0.5);
    assert!(matches!(tall, SplitLayout::Side { .. }), "{tall:?}");
    let short = layout(vec2(900.0, 400.0), Orientation::SideBySide, 0.5);
    assert_eq!(short, SplitLayout::Single);
}

#[test]
fn the_named_minimums_are_the_staircase_extremes() {
    assert_eq!(
        STAIRS.iter().map(|s| s.0).fold(f32::MAX, f32::min),
        MIN_PANE_WIDTH
    );
    assert_eq!(
        STAIRS.iter().map(|s| s.1).fold(f32::MAX, f32::min),
        MIN_PANE_HEIGHT
    );
    // A wider corner is never higher than a narrower one (the table is a staircase).
    assert!(
        STAIRS
            .windows(2)
            .all(|p| p[0].0 < p[1].0 && p[0].1 > p[1].1)
    );
}

#[test]
fn stacked_at_1000x700_has_room_for_a_sidebar_a_little_wider() {
    // The central area is 1000 less the sidebar, the margins and the 2 pt rings: the measured
    // corner (695, 280) leaves 3 pt at the default 280 pt sidebar, so it holds up to 283 pt.
    for sidebar in [280.0, 282.0, 283.0] {
        let central = vec2(1000.0 - sidebar - 16.0, 700.0 - 16.0 - 32.0);
        let got = layout(central, Orientation::Stacked, 0.5);
        assert!(
            matches!(got, SplitLayout::Stacked { .. }),
            "{sidebar}: {got:?}"
        );
    }
}

#[test]
fn the_layout_does_not_depend_on_what_it_chose() {
    // No feedback: the same area gives the same layout however often it is asked.
    let central = vec2(704.0, 652.0);
    let first = layout(central, Orientation::SideBySide, 0.5);
    assert_eq!(first, layout(central, Orientation::SideBySide, 0.5));
}

#[test]
fn the_divider_limits_are_what_the_other_pane_leaves() {
    let (lo, hi) = ratio_limits(vec2(2408.0, 900.0), Orientation::SideBySide);
    assert_eq!((lo, hi), (0.25, 0.75));
    let (lo, hi) = ratio_limits(central(1400.0, 900.0), Orientation::SideBySide);
    assert!(
        lo > 0.25 && hi < 0.75 && (lo + hi - 1.0).abs() < 1e-5,
        "{lo} {hi}"
    );
    assert_eq!(
        ratio_limits(central(800.0, 500.0), Orientation::SideBySide),
        (0.25, 0.75)
    );
}
