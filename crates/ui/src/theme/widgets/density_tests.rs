//! The theme's control density: every single-line control is 28 pt, the field's text fits inside
//! it with room to spare, and a focused control's ring does not reach into the row below it.
use super::widgets_tests::themed;
use super::{
    CONTROL_HEIGHT, outlined_field, ring_extent, ring_outer, segmented, tabs, text_button,
};
use eframe::egui::{Rect, TextStyle, accesskit::Role};
use egui_kittest::kittest::Queryable;

/// A desktop-dense 28 pt, as the spec decided.
const DENSE_HEIGHT: f32 = 28.0;
/// The ring stops at least this far short of the next row (points).
const MIN_RING_MARGIN: f32 = 1.0;
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
    // A strictly positive margin: the ring ends before the lower button begins, with room to spare.
    let margin = lower.top() - ring.bottom();
    assert!(
        margin >= MIN_RING_MARGIN,
        "the ring {ring:?} leaves {margin} pt above the lower button {lower:?}: the row gap is too small"
    );
}

#[test]
fn the_painted_ring_ends_where_ring_outer_says() {
    use super::{Look, Ring, focus_ring};
    use eframe::egui::{Color32, CornerRadius, Shape, pos2, vec2};
    let control = Rect::from_min_size(pos2(50.0, 50.0), vec2(100.0, super::CONTROL_HEIGHT));
    let harness = themed(300.0, (), move |ui, _: &mut ()| {
        let look = Look {
            layer: None,
            focused: true,
            enabled: true,
        };
        focus_ring(
            ui.painter(),
            control,
            CornerRadius::same(8),
            look,
            Color32::GREEN,
            Ring::Outside,
        );
    });
    let painted = harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Rect(r) if r.stroke.width > 0.0 => Some(r.rect.expand(r.stroke.width)),
            _ => None,
        })
        .next()
        .expect("the ring is painted");
    assert_eq!(painted, ring_outer(control));
}

/// The lines of `source` that set a row gap (`item_spacing.y = N`) smaller than `extent` without
/// saying `ring-gap-exempt` on that line or the one before it.
fn small_row_gaps(source: &str, extent: f32) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(at) = line.find("item_spacing.y = ") else {
            continue;
        };
        let value = line[at + "item_spacing.y = ".len()..]
            .trim_end_matches(';')
            .trim();
        let Ok(gap) = value.parse::<f32>() else {
            continue;
        };
        let exempt =
            line.contains("ring-gap-exempt") || (i > 0 && lines[i - 1].contains("ring-gap-exempt"));
        if gap < extent && !exempt {
            found.push(format!("{}: {}", i + 1, line.trim()));
        }
    }
    found
}

fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read the source directory") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_scanner_finds_a_row_gap_below_the_ring_and_honours_the_exemption() {
    assert_eq!(
        small_row_gaps("ui.spacing_mut().item_spacing.y = 0.0;", 3.0).len(),
        1
    );
    assert!(small_row_gaps("ui.spacing_mut().item_spacing.y = 4.0;", 3.0).is_empty());
    let exempt = "// ring-gap-exempt: text only\nui.spacing_mut().item_spacing.y = 0.0;";
    assert!(small_row_gaps(exempt, 3.0).is_empty());
}

#[test]
fn no_view_sets_a_local_row_gap_below_the_focus_ring() {
    let mut files = Vec::new();
    rust_files(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut offenders = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file).expect("read a source file");
        for line in small_row_gaps(&source, ring_extent()) {
            offenders.push(format!("{}:{line}", file.display()));
        }
    }
    assert!(
        offenders.is_empty(),
        "a row gap below the focus ring's {} pt reach: mark it `ring-gap-exempt` with a reason or widen it: {offenders:#?}",
        ring_extent()
    );
}
