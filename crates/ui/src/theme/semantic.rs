//! Colors for content that is not chrome: JSON syntax and chart series. Hand-picked per mode in
//! Material's tonal spirit (tone 40 in light, tone 80 in dark) and tied to the scheme's roles
//! where a role fits; `semantic_tests` arbitrates the contrast (JSON 4.5:1 and chart series 3:1
//! against both `surface` and `surface_container_low`).
use super::{Mode, Scheme};
use eframe::egui::Color32;

/// Syntax colors for one line of JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JsonColors {
    pub key: Color32,
    pub string: Color32,
    pub number: Color32,
    pub boolean: Color32,
    pub null: Color32,
    pub punctuation: Color32,
}

/// Colors of the usage and miss charts. A series is never told apart by color alone: the legend
/// names it and bars, lines and bands differ in shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChartColors {
    pub series: [Color32; 4],
    /// The band over time nobody observed.
    pub no_data: Color32,
    /// The line where the server restarted.
    pub restart: Color32,
    /// Grid and axis lines. The plot library takes these from the egui visuals, which `install`
    /// builds from the same role (`outline_variant`); this is the value to compare against.
    pub grid: Color32,
}

const fn rgb(h: u32) -> Color32 {
    Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

/// JSON syntax colors for `scheme`.
pub fn json_colors(scheme: &Scheme) -> JsonColors {
    let (key, number, boolean, null) = match scheme.mode() {
        Mode::Light => (rgb(0x1B5E9B), rgb(0x8A4B00), rgb(0x7A3F9A), rgb(0x9C3560)),
        Mode::Dark => (rgb(0x9CC9FF), rgb(0xF7BD48), rgb(0xDDB8F5), rgb(0xF5A8C6)),
    };
    JsonColors {
        key,
        string: scheme.success,
        number,
        boolean,
        null,
        punctuation: scheme.on_surface_variant,
    }
}

/// Chart colors for `scheme`: primary green, warning amber, then a blue and a purple.
pub fn chart_colors(scheme: &Scheme) -> ChartColors {
    let (blue, purple) = match scheme.mode() {
        Mode::Light => (rgb(0x2F5F9E), rgb(0x7B4F9C)),
        Mode::Dark => (rgb(0x9CC3FF), rgb(0xD2B0F0)),
    };
    ChartColors {
        series: [scheme.primary, scheme.warning, blue, purple],
        no_data: scheme.outline,
        restart: scheme.error,
        grid: scheme.outline_variant,
    }
}
