//! History panel state of the Shell tab: the remembered runs of the connection, and the panel's
//! open and confirm-clear flags. Pure and egui-free; every updater returns a new value.
//! No user-facing strings.
use super::shell::ShellState;
pub use curlybrace_core::history::ShellHistoryEntry;
use std::sync::Arc;

/// The history list of one shell tab. `loaded` and `available` give three states:
/// - not loaded (`loaded == false`): no answer from the core yet, the view shows "loading";
/// - unavailable (`loaded && !available`): history is off or its store cannot be used;
/// - available (`loaded && available`): `entries` (newest first, possibly empty) is the list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryView {
    /// The core has answered at least once (with a list or with "unavailable").
    pub loaded: bool,
    pub available: bool,
    pub open: bool,
    pub entries: Arc<Vec<ShellHistoryEntry>>,
    /// The "clear history" button was pressed and awaits confirmation.
    pub confirm_clear: bool,
}

impl ShellState {
    fn with_history(&self, history: HistoryView) -> Self {
        Self {
            history,
            ..self.clone()
        }
    }

    pub fn history_loaded(&self, entries: Vec<ShellHistoryEntry>) -> Self {
        self.with_history(HistoryView {
            loaded: true,
            available: true,
            entries: Arc::new(entries),
            ..self.history.clone()
        })
    }

    pub fn history_unavailable(&self) -> Self {
        self.with_history(HistoryView {
            loaded: true,
            available: false,
            entries: Arc::default(),
            ..self.history.clone()
        })
    }

    pub fn history_toggled(&self) -> Self {
        self.with_history(HistoryView {
            open: !self.history.open,
            ..self.history.clone()
        })
    }

    /// Puts the entry's script in the editor (as `edited` does: no selection), also mid-run.
    /// An unknown id changes nothing.
    pub fn history_entry_recalled(&self, id: i64) -> Self {
        match self.history.entries.iter().find(|e| e.id == id) {
            Some(entry) => self.edited(entry.script.clone(), None),
            None => self.clone(),
        }
    }

    pub fn clear_requested(&self) -> Self {
        self.with_confirm_clear(true)
    }

    pub fn clear_cancelled(&self) -> Self {
        self.with_confirm_clear(false)
    }

    /// Resets the confirm step; the flag is true only if a clear was pending, i.e. the caller
    /// should send the command. The list empties when the core's answer arrives.
    pub fn clear_confirmed(&self) -> (Self, bool) {
        (self.with_confirm_clear(false), self.history.confirm_clear)
    }

    fn with_confirm_clear(&self, confirm_clear: bool) -> Self {
        self.with_history(HistoryView {
            confirm_clear,
            ..self.history.clone()
        })
    }
}

/// The first non-blank line of `script`, trimmed, cut to `max_chars` characters with `…` appended.
pub fn first_line(script: &str, max_chars: usize) -> String {
    let line = script
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    match line.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    }
}

#[cfg(test)]
#[path = "shell_history_tests.rs"]
mod tests;
