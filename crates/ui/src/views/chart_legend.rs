//! The legend entries of the charts: what each drawn series is called and the color it is drawn
//! in. A color is never the only signal; every entry carries its name, so the legend (and the
//! text summary under the chart) tell the series apart without hue.
use crate::I18n;
use crate::theme::ChartColors;
use eframe::egui::Color32;

/// One legend row: the series' translated name and its color.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegendEntry {
    pub name: String,
    pub color: Color32,
}

fn entry(i18n: &I18n, key: &str, color: Color32) -> LegendEntry {
    LegendEntry {
        name: i18n.t(key),
        color,
    }
}

/// The legend of an index's usage chart: uses, server restarts and unobserved time.
pub fn index_legend(i18n: &I18n, colors: &ChartColors) -> [LegendEntry; 3] {
    [
        entry(i18n, "indexes.uses_series", colors.series[0]),
        entry(i18n, "indexes.restart", colors.restart),
        entry(i18n, "indexes.no_data", colors.no_data),
    ]
}

/// The legend of the Queries miss trend: misses and unobserved time.
pub fn queries_legend(i18n: &I18n, colors: &ChartColors) -> [LegendEntry; 2] {
    [
        entry(i18n, "indexes.queries.misses_series", colors.series[1]),
        entry(i18n, "indexes.no_data", colors.no_data),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Mode, chart_colors, scheme};

    #[test]
    fn series_stay_distinguishable_by_label_not_only_hue() {
        let i18n = I18n::english();
        for mode in [Mode::Light, Mode::Dark] {
            let colors = chart_colors(scheme(mode));
            let index = index_legend(&i18n, &colors);
            let names: Vec<_> = index.iter().map(|e| e.name.as_str()).collect();
            assert_eq!(names, ["Uses", "Server restarted", "No data"], "{mode:?}");
            let queries = queries_legend(&i18n, &colors);
            assert_eq!(queries[0].name, i18n.t("indexes.queries.misses_series"));
            assert!(
                queries
                    .iter()
                    .chain(&index)
                    .all(|e| !e.name.contains('\u{27e6}'))
            );
            for legend in [index.to_vec(), queries.to_vec()] {
                for (i, a) in legend.iter().enumerate() {
                    assert!(!a.name.is_empty());
                    for b in &legend[i + 1..] {
                        assert_ne!(a.name, b.name);
                        assert_ne!(a.color, b.color, "{mode:?}");
                    }
                }
            }
        }
    }
}
