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
    // Room for the editor's share, the longest list and the output's share.
    let h = heights(4000.0, true, 200.0);
    assert_eq!(h.editor, 4000.0 * 0.45);
    assert_eq!(h.list, LIST_MAX_HEIGHT);
}

#[test]
fn the_open_list_is_capped_to_a_share_of_the_height() {
    for available in [300.0, 600.0, 700.0] {
        let h = heights(available, true, 0.0);
        assert!(h.list <= available * 0.25, "{available}: {h:?}");
    }
}

#[test]
fn open_the_output_is_planned_its_share_when_the_floors_allow() {
    // 1280x800 and 800x500 windows (the tab gets about 700 and 400 points) with their chrome.
    for (available, chrome) in [(702.0, 120.0), (402.0, 110.0)] {
        let h = heights(available, true, chrome);
        let output = available - chrome - SPACING_SLACK - h.editor - h.list;
        let at_floors = h.editor == EDITOR_MIN_OPEN_HEIGHT && h.list == LIST_MIN_HEIGHT;
        assert!(
            output >= available * OUTPUT_OPEN_SHARE - 0.01 || at_floors,
            "{available}: {h:?} {output}"
        );
        assert!(output >= OUTPUT_MIN_HEIGHT, "{available}: {h:?} {output}");
    }
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
