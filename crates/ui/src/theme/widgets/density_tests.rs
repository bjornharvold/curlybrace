//! The theme's control density: every single-line control is 28 pt, the field's text fits inside
//! it with room to spare, and a focused control's ring does not reach into the row below it.
use super::widgets_tests::themed;
use super::{CONTROL_HEIGHT, outlined_field, ring_outer, segmented, tabs, text_button};
use eframe::egui::{Rect, TextStyle, accesskit::Role};
use egui_kittest::kittest::Queryable;

/// A desktop-dense 28 pt, as the spec decided.
const DENSE_HEIGHT: f32 = 28.0;
/// A field's body text keeps at least this much room above and below it.
const MIN_TEXT_PADDING: f32 = 4.0;

#[test]
fn the_control_height_is_28_points() {
    assert_eq!(CONTROL_HEIGHT, DENSE_HEIGHT);
}

#[test]
fn a_button_a_tab_and_a_segmented_option_are_28_points_tall() {
    let tab_items = vec![(0u8, "Documents".to_owned()), (1u8, "Indexes".to_owned())];
    let segments = vec![(0u8, "One".to_owned()), (1u8, "Two".to_owned())];
    let harness = themed(500.0, (), move |ui, _: &mut ()| {
        text_button(ui, "Button");
        tabs(ui, &tab_items, 0);
        segmented(ui, &segments, 0);
    });
    let button = harness.get_by_role_and_label(Role::Button, "Button");
    assert_eq!(button.rect().height(), DENSE_HEIGHT, "button");
    let tab = harness.get_by_label("Documents");
    assert_eq!(tab.rect().height(), DENSE_HEIGHT, "tab");
    let segment = harness.get_by_role_and_label(Role::RadioButton, "One");
    assert_eq!(segment.rect().height(), DENSE_HEIGHT, "segmented option");
}

/// What a single-line field measured in one frame.
#[derive(Clone, Copy)]
struct FieldSeen {
    field: Rect,
    body: f32,
    overhang: f32,
    small: f32,
}

#[test]
fn a_single_line_field_is_28_points_and_its_text_has_room_above_and_below() {
    let mut harness = themed(400.0, None, |ui, seen: &mut Option<FieldSeen>| {
        let mut text = String::new();
        let top = ui.cursor().top();
        let field = outlined_field(ui, "Name", &mut text, "").rect;
        *seen = Some(FieldSeen {
            field,
            body: ui.text_style_height(&TextStyle::Body),
            overhang: field.top() - top,
            small: ui.text_style_height(&TextStyle::Small),
        });
    });
    harness.run();
    let seen = harness.state().expect("ran");
    assert_eq!(seen.field.height(), DENSE_HEIGHT);
    let padding = (seen.field.height() - seen.body) / 2.0;
    assert!(
        padding >= MIN_TEXT_PADDING,
        "the text row ({}) leaves {padding} above and below, not {MIN_TEXT_PADDING}",
        seen.body
    );
    // The floating label's overhang above the border is still half the small text.
    assert!(
        (seen.overhang - seen.small / 2.0).abs() < 0.5,
        "the label overhangs by {} instead of {}",
        seen.overhang,
        seen.small / 2.0
    );
}

#[test]
fn a_focused_buttons_ring_does_not_overlap_the_button_below_it() {
    let mut harness = themed(400.0, None, |ui, seen: &mut Option<(Rect, Rect)>| {
        let upper = text_button(ui, "Upper");
        let lower = text_button(ui, "Lower");
        *seen = Some((upper.rect, lower.rect));
    });
    harness.run();
    let (upper, lower) = harness.state().expect("ran");
    let ring = ring_outer(upper);
    let overlap = ring.intersect(lower);
    let area = overlap.width().max(0.0) * overlap.height().max(0.0);
    assert!(
        area <= 0.0,
        "the ring {ring:?} overlaps the lower button {lower:?} by {overlap:?}: the row gap is too small"
    );
}
