//! The theme widgets laid out for the gallery screenshot and the keyboard-focus test. Included
//! with `#[path]` by `theme_gallery.rs` and `accessibility_theme_widgets.rs` (not through
//! `support/mod.rs`, so neither target compiles the app fixtures). Labels are plain test strings.
#![allow(dead_code)]

use curlybrace_ui::theme::widgets::{Forced, with_forced};
use curlybrace_ui::theme::{
    self, Mode, Tone, card, chip, filled_button, outlined_field, pill, segmented, text_button,
    tonal_button,
};
use eframe::egui;

/// The labels of the interactive widgets, in Tab order.
pub const TAB_ORDER: &[&str] = &[
    "Browse",
    "Compare",
    "customers",
    "invoices",
    "Name",
    "Find",
    "Clear",
    "Cancel",
];

/// What the widgets edit.
pub struct GalleryState {
    pub view: u8,
    pub collection: u8,
    pub name: String,
}

impl Default for GalleryState {
    fn default() -> Self {
        Self {
            view: 0,
            collection: 0,
            name: "Production".to_owned(),
        }
    }
}

/// Installs the theme and switches to `mode` on the first frame, then draws `body`.
pub fn themed<S>(
    mode: Mode,
    mut body: impl FnMut(&mut egui::Ui, &mut S),
) -> impl FnMut(&mut egui::Ui, &mut S) {
    let mut installed = false;
    move |ui, state| {
        if !installed {
            theme::install(ui.ctx());
            ui.ctx().set_theme(match mode {
                Mode::Light => egui::Theme::Light,
                Mode::Dark => egui::Theme::Dark,
            });
            installed = true;
        }
        body(ui, state);
    }
}

fn views() -> Vec<(u8, String)> {
    vec![(0, "Browse".to_owned()), (1, "Compare".to_owned())]
}

/// One of each interactive widget, enabled, in [`TAB_ORDER`].
pub fn interactive(ui: &mut egui::Ui, state: &mut GalleryState) {
    if let Some(v) = segmented(ui, &views(), state.view) {
        state.view = v;
    }
    for (i, name) in [(0u8, "customers"), (1, "invoices")] {
        if pill(ui, state.collection == i, name).clicked() {
            state.collection = i;
        }
    }
    outlined_field(ui, "Name", &mut state.name, "e.g. Production");
    ui.horizontal(|ui| {
        filled_button(ui, "Find");
        tonal_button(ui, "Clear");
        text_button(ui, "Cancel");
    });
}

/// A column of the gallery: a widget state forced on every widget in it.
#[derive(Clone, Copy)]
enum Column {
    Enabled,
    Forced(Forced),
    Disabled,
}

const COLUMNS: &[(&str, Column)] = &[
    ("enabled", Column::Enabled),
    ("hover", Column::Forced(Forced::Hover)),
    ("pressed", Column::Forced(Forced::Pressed)),
    ("focused", Column::Forced(Forced::Focused)),
    ("disabled", Column::Disabled),
];

const COLUMN_WIDTH: f32 = 170.0;
const LONG: &str = "A very long label that has to be cut short with an ellipsis";

type Row = (&'static str, fn(&mut egui::Ui, &mut GalleryState));

const ROWS: &[Row] = &[
    ("filled", |ui, _| {
        filled_button(ui, "Save");
    }),
    ("tonal", |ui, _| {
        tonal_button(ui, "Clear");
    }),
    ("text", |ui, _| {
        text_button(ui, "Cancel");
    }),
    ("segmented", |ui, s| {
        segmented(ui, &views(), s.view);
    }),
    ("pill on", |ui, _| {
        pill(ui, true, "customers");
    }),
    ("pill off", |ui, _| {
        pill(ui, false, "invoices");
    }),
    ("field", |ui, s| {
        outlined_field(ui, "Name", &mut s.name, "e.g. Production");
    }),
    ("field hint", |ui, _| {
        outlined_field(ui, "Database", &mut String::new(), "e.g. crm");
    }),
    ("long", |ui, _| {
        filled_button(ui, LONG);
    }),
];

/// Every widget in every state: one row per widget, one column per state, then the chips in
/// each tone (the error state is the `Bad` tone) and a card.
pub fn gallery(ui: &mut egui::Ui, state: &mut GalleryState) {
    egui::Grid::new("theme_gallery")
        .num_columns(COLUMNS.len() + 1)
        .min_col_width(COLUMN_WIDTH)
        .max_col_width(COLUMN_WIDTH)
        .spacing(egui::vec2(12.0, 12.0))
        .show(ui, |ui| {
            ui.label("");
            for (name, _) in COLUMNS {
                ui.label(*name);
            }
            ui.end_row();
            for (name, draw) in ROWS {
                ui.label(*name);
                for (_, column) in COLUMNS {
                    cell(ui, *column, state, *draw);
                }
                ui.end_row();
            }
        });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        for (tone, text) in [
            (Tone::Good, "Uses index"),
            (Tone::Warning, "Partial"),
            (Tone::Bad, "Collection scan"),
            (Tone::Neutral, "Unknown"),
        ] {
            chip(ui, tone, text);
        }
        ui.allocate_ui(egui::vec2(120.0, 24.0), |ui| chip(ui, Tone::Bad, LONG));
    });
    ui.add_space(12.0);
    ui.allocate_ui(egui::vec2(COLUMN_WIDTH * 3.0, 200.0), |ui| {
        card(ui, |ui| {
            ui.label("A card holds related content");
            ui.horizontal(|ui| {
                filled_button(ui, "Run");
                text_button(ui, "Details");
            });
        })
    });
}

fn cell(
    ui: &mut egui::Ui,
    column: Column,
    state: &mut GalleryState,
    draw: fn(&mut egui::Ui, &mut GalleryState),
) {
    ui.push_id(ui.next_auto_id(), |ui| {
        ui.set_width(COLUMN_WIDTH);
        match column {
            Column::Enabled => draw(ui, state),
            Column::Forced(forced) => with_forced(ui, forced, |ui| draw(ui, state)),
            Column::Disabled => {
                ui.add_enabled_ui(false, |ui| draw(ui, state));
            }
        }
    });
}
