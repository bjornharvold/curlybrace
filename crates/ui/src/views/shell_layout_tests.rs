use super::*;

#[test]
fn closed_keeps_the_usual_editor_share_whatever_the_chrome() {
    for available in [200.0, 362.0, 662.0, 1200.0] {
        let expected = (available * 0.45_f32).max(120.0);
        for chrome in [0.0, 220.0, 900.0] {
            let h = heights(available, false, chrome);
            assert_eq!(
                h,
                Heights {
                    editor: expected,
                    list: 0.0
                },
                "{available}"
            );
        }
    }
}

#[test]
fn open_with_room_to_spare_changes_nothing_for_the_editor() {
    let h = heights(1200.0, true, 200.0);
    assert_eq!(h.editor, 1200.0 * 0.45);
    assert_eq!(h.list, LIST_MAX_HEIGHT);
}

#[test]
fn the_open_list_is_capped_to_a_share_of_the_height() {
    let h = heights(600.0, true, 0.0);
    assert_eq!(h.list, 150.0);
}

#[test]
fn when_tight_the_editor_and_list_shrink_so_the_output_keeps_its_minimum() {
    for (available, chrome) in [
        (340.0, 100.0),
        (420.0, 180.0),
        (500.0, 220.0),
        (662.0, 240.0),
    ] {
        let h = heights(available, true, chrome);
        assert!(h.editor >= EDITOR_MIN_OPEN_HEIGHT, "{h:?}");
        assert!(h.list >= LIST_MIN_HEIGHT, "{h:?}");
        let output = available - chrome - SPACING_SLACK - h.editor - h.list;
        assert!(
            output >= OUTPUT_MIN_HEIGHT - 0.01,
            "{available}: {h:?} {output}"
        );
    }
}

#[test]
fn past_the_floors_the_output_gives_way() {
    let h = heights(250.0, true, 200.0);
    assert_eq!(
        h,
        Heights {
            editor: EDITOR_MIN_OPEN_HEIGHT,
            list: LIST_MIN_HEIGHT
        }
    );
}
