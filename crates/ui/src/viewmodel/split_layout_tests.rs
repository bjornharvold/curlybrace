use super::{
    DIVIDER, MIN_PANE_HEIGHT, MIN_PANE_WIDTH, Orientation, STAIRS, SplitLayout, clamped, layout,
    min_height_at, min_width_at,
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
fn side_by_side_is_the_default_at_1000x700_because_the_measured_page_fits() {
    // Two 348 pt wide panes 652 high keep the page usable (the staircase has (250, 560)).
    let got = layout(central(1000.0, 700.0), Orientation::SideBySide, 0.5);
    assert_eq!(got, SplitLayout::Side { ratio: 0.5 });
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
    let got = layout(central(1700.0, 900.0), Orientation::SideBySide, 0.99);
    assert_eq!(got, SplitLayout::Side { ratio: 0.75 });
}

#[test]
fn the_ratio_never_squeezes_a_pane_below_its_minimum() {
    // Just wide enough for two panes of the narrowest width that works at this height (470 at
    // 400 high): the divider cannot move at all.
    let narrow = vec2(2.0 * 470.0 + DIVIDER, 400.0);
    assert_eq!(
        layout(narrow, Orientation::SideBySide, 0.25),
        SplitLayout::Side { ratio: 0.5 }
    );
}

#[test]
fn a_narrower_pane_needs_a_taller_one() {
    assert_eq!(min_width_at(600.0), Some(250.0));
    assert_eq!(min_width_at(400.0), Some(470.0));
    assert_eq!(min_width_at(200.0), None);
    assert_eq!(min_height_at(250.0), Some(560.0));
    assert_eq!(min_height_at(1000.0), Some(250.0));
    assert_eq!(min_height_at(200.0), None);
    // 316 wide panes fit when they are 600 high, not when they are 400 high.
    let tall = layout(vec2(640.0, 600.0), Orientation::SideBySide, 0.5);
    assert!(matches!(tall, SplitLayout::Side { .. }), "{tall:?}");
    let short = layout(vec2(640.0, 400.0), Orientation::SideBySide, 0.5);
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
