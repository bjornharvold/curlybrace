use super::*;

const GAP: f32 = 8.0;

#[test]
fn at_1000_both_json_fields_keep_their_minimum_and_the_row_is_one_row() {
    for toggle in [134.0, 150.0] {
        let json = json_width(703.0, GAP, toggle);
        assert!(
            json >= JSON_FIELD_MIN_WIDTH - 0.5,
            "toggle {toggle}: {json}"
        );
        let row = 2.0 * json + 2.0 * NUMBER_FIELD_WIDTH + toggle + 4.0 * GAP;
        assert!(row <= 703.0, "toggle {toggle}: the row is {row} wide");
    }
}

#[test]
fn what_skip_limit_and_the_toggle_leave_is_shared_by_sort_and_projection() {
    let json = json_width(703.0, GAP, 134.0);
    let leftover = 703.0 - 2.0 * NUMBER_FIELD_WIDTH - 134.0 - 4.0 * GAP;
    // Within the slack and the rounding down.
    assert!(
        (0.0..=3.0).contains(&(leftover - 2.0 * json)),
        "{json} {leftover}"
    );
}

#[test]
fn when_the_row_cannot_hold_everything_sort_and_projection_take_half_a_row_each() {
    let half = (504.0 - GAP) / 2.0;
    let json = json_width(504.0, GAP, 134.0);
    assert!(
        (half - 2.0..=half).contains(&json),
        "{json} for half {half}"
    );
    // The two never add up to more than the row, so Projection does not wrap on its own.
    let json = json_width(503.0, GAP, 134.0);
    assert!(2.0 * json + GAP <= 503.0, "{json}");
}

#[test]
fn a_field_is_never_wider_than_the_row_nor_narrower_than_the_minimum_unless_the_row_is() {
    assert_eq!(json_width(300.0, GAP, 134.0), JSON_FIELD_MIN_WIDTH);
    assert_eq!(json_width(120.0, GAP, 134.0), 120.0);
    for available in [120.0, 300.0, 504.0, 703.0, 2000.0] {
        let json = json_width(available, GAP, 134.0);
        assert!(json <= available, "{available}: {json}");
        assert!(
            json >= JSON_FIELD_MIN_WIDTH.min(available),
            "{available}: {json}"
        );
    }
}
