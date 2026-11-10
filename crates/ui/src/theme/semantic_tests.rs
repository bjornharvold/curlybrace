use super::contrast::ratio;
use super::{Mode, chart_colors, js_colors, json_colors, scheme};

const MODES: [Mode; 2] = [Mode::Light, Mode::Dark];

#[test]
fn chart_series_reach_3_to_1_in_both_modes() {
    for mode in MODES {
        let s = scheme(mode);
        let c = chart_colors(s);
        let mut drawn = c.series.to_vec();
        drawn.extend([c.no_data, c.restart]);
        for (i, color) in drawn.into_iter().enumerate() {
            for (name, bg) in [
                ("surface_container_low", s.surface_container_low),
                ("surface", s.surface),
            ] {
                let r = ratio(color, bg);
                assert!(r >= 3.0, "{mode:?} #{i} on {name}: {r:.2}");
            }
        }
    }
}

#[test]
fn chart_series_never_share_a_color() {
    for mode in MODES {
        let c = chart_colors(scheme(mode));
        let mut all = c.series.to_vec();
        all.extend([c.no_data, c.restart]);
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b, "{mode:?}");
            }
        }
    }
}

#[test]
fn json_colors_reach_aa_on_the_surface() {
    for mode in MODES {
        let s = scheme(mode);
        let j = json_colors(s);
        let tokens = [
            ("key", j.key),
            ("string", j.string),
            ("number", j.number),
            ("boolean", j.boolean),
            ("null", j.null),
            ("punctuation", j.punctuation),
        ];
        for (name, color) in tokens {
            for (bg_name, bg) in [
                ("surface_container_low", s.surface_container_low),
                ("surface", s.surface),
            ] {
                let r = ratio(color, bg);
                assert!(r >= 4.5, "{mode:?} {name} on {bg_name}: {r:.2}");
            }
        }
    }
}

#[test]
fn js_colors_reach_aa_on_the_surface() {
    for mode in MODES {
        let s = scheme(mode);
        let j = js_colors(s);
        let tokens = [
            ("plain", j.plain),
            ("keyword", j.keyword),
            ("string", j.string),
            ("number", j.number),
            ("comment", j.comment),
            ("shell_word", j.shell_word),
        ];
        for (name, color) in tokens {
            for (bg_name, bg) in [
                ("surface_container_low", s.surface_container_low),
                ("surface", s.surface),
            ] {
                let r = ratio(color, bg);
                assert!(r >= 4.5, "{mode:?} {name} on {bg_name}: {r:.2}");
            }
        }
    }
}

#[test]
fn js_token_kinds_are_told_apart() {
    for mode in MODES {
        let j = js_colors(scheme(mode));
        let kinds = [
            j.plain,
            j.keyword,
            j.string,
            j.number,
            j.comment,
            j.shell_word,
        ];
        for (i, a) in kinds.iter().enumerate() {
            for b in &kinds[i + 1..] {
                assert_ne!(a, b, "{mode:?}");
            }
        }
    }
}

#[test]
fn json_value_kinds_are_told_apart() {
    for mode in MODES {
        let j = json_colors(scheme(mode));
        let kinds = [j.key, j.string, j.number, j.boolean, j.null];
        for (i, a) in kinds.iter().enumerate() {
            for b in &kinds[i + 1..] {
                assert_ne!(a, b, "{mode:?}");
            }
        }
    }
}
