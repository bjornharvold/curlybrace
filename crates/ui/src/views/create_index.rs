//! Whether a Create index... button that opens the index builder works, and why not. Shared by
//! the Queries rows and the Browse verdict popup so both give the same reasons.

use super::findbar::described;
use crate::I18n;
use crate::theme::{gate, tonal_button};
use eframe::egui;

/// The state of a Create index... button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateIndex {
    /// No button (the guard notices: their context is "about to run", not the collection).
    Hidden,
    On,
    /// Off: the connection is read-only.
    ReadOnly,
    /// Off: an index write is waiting for the server.
    IndexWrite,
    /// Off: another dialog of the index or profiler flows is open.
    Prompt,
    /// Off: a document write is parked for the production warning.
    WriteWarning,
}

/// What can turn Create index... off, by name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CreateBlockers {
    /// The connection is read-only.
    pub read_only: bool,
    /// An index write is waiting for the server.
    pub index_write: bool,
    /// Another dialog of the index or profiler flows is open.
    pub prompt: bool,
    /// A document write is parked for the production warning.
    pub write_warning: bool,
}

impl CreateIndex {
    /// The state for these blockers, the first that applies winning: read-only, an index write in
    /// flight, a parked write warning, another open dialog.
    pub fn of(blockers: CreateBlockers) -> Self {
        let CreateBlockers {
            read_only,
            index_write,
            prompt,
            write_warning,
        } = blockers;
        match (read_only, index_write, write_warning, prompt) {
            (true, ..) => Self::ReadOnly,
            (_, true, ..) => Self::IndexWrite,
            (_, _, true, _) => Self::WriteWarning,
            (_, _, _, true) => Self::Prompt,
            _ => Self::On,
        }
    }

    /// Read-only or nothing in the way, for a view with no other blocker to report.
    pub fn read_only(read_only: bool) -> Self {
        Self::of(CreateBlockers {
            read_only,
            ..CreateBlockers::default()
        })
    }

    /// Why the button is off (its `en.json` key); `None` while it works or is hidden.
    pub fn off_reason(self) -> Option<&'static str> {
        match self {
            Self::Hidden | Self::On => None,
            Self::ReadOnly => Some("indexes.create_suggested_read_only"),
            Self::IndexWrite => Some("indexes.write_busy"),
            Self::Prompt => Some("indexes.create_prompt_open"),
            Self::WriteWarning => Some("indexes.create_warning_open"),
        }
    }
}

/// The Create index... button in `state`. `name` is its accessible name when that must say more
/// than the drawn text (which query it is for); the drawn text is the same everywhere. Off, it
/// keeps its place and gives the reason as hover text and accessible description. Returns whether
/// it was clicked while on.
pub(super) fn create_button(
    ui: &mut egui::Ui,
    state: CreateIndex,
    name: Option<&str>,
    i18n: &I18n,
) -> bool {
    let off = state.off_reason();
    let response = gate(ui, off.is_none(), |ui| {
        tonal_button(ui, i18n.t("indexes.create_suggested"))
    });
    if let Some(name) = name {
        let name = name.to_string();
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_label(name));
    }
    match off {
        Some(key) => {
            described(ui, response, i18n.t(key));
            false
        }
        None => response.clicked(),
    }
}

#[cfg(test)]
#[path = "create_index_tests.rs"]
mod tests;
