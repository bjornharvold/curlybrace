//! View-model of the Shell tab: the script, its selection, the captured output and the run state.
//! Pure and egui-free; every updater returns a new value and never mutates. No user-facing strings.
use super::shell_format::format_script;
use super::shell_history::HistoryView;
pub use super::shell_output::{MAX_LINE_BYTES, MAX_OUTPUT_LINES, OutputLine, OutputLog};
use super::shell_result::{OutputMode, ResultView};
use curlybrace_core::runtime::ShellRefusal;
use curlybrace_core::shell::{Outcome, Stream};
use curlybrace_core::tools::{ServerVersion, ToolError, ToolVersion};
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShellStatus {
    #[default]
    Idle,
    Running {
        run_id: u64,
    },
}

/// Why the shell tool cannot be used for a run.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolProblem {
    Missing(ToolError),
    TooOld {
        tool: ToolVersion,
        server: ServerVersion,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShellNotice {
    Finished(Outcome),
    Refused(ShellRefusal),
    ToolProblem(ToolProblem),
    EmptyScript,
}

/// What to run: the selection, else the whole script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunRequest {
    pub run_id: u64,
    pub script: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShellState {
    pub script: String,
    pub selection: Option<Range<usize>>,
    pub output: OutputLog,
    /// Lines the `MAX_OUTPUT_LINES` window discarded since the run started.
    pub dropped_lines: u64,
    pub status: ShellStatus,
    pub next_run_id: u64,
    pub notice: Option<ShellNotice>,
    /// The remembered runs of this connection and the history panel's flags.
    pub history: HistoryView,
    /// How the output is shown; chosen by the person and kept across runs.
    pub mode: OutputMode,
    /// The last value of the current (or last) run, for the Table and Tree modes.
    pub result: ResultView,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            script: String::new(),
            selection: None,
            output: OutputLog::default(),
            dropped_lines: 0,
            status: ShellStatus::Idle,
            next_run_id: 1,
            notice: None,
            history: HistoryView::default(),
            mode: OutputMode::Text,
            result: ResultView::default(),
        }
    }
}

impl ShellState {
    pub fn is_running(&self) -> bool {
        matches!(self.status, ShellStatus::Running { .. })
    }

    /// Whether the Run button is enabled: idle and the script has non-whitespace text.
    pub fn can_run(&self) -> bool {
        !self.is_running() && !self.script.trim().is_empty()
    }

    pub(super) fn is_current(&self, run_id: u64) -> bool {
        self.status == (ShellStatus::Running { run_id })
    }

    pub fn edited(&self, script: String, selection: Option<Range<usize>>) -> Self {
        Self {
            script,
            selection,
            ..self.clone()
        }
    }

    pub fn formatted(&self) -> Self {
        let result = format_script(&self.script, self.effective_selection());
        Self {
            script: result.text,
            selection: result.selection,
            ..self.clone()
        }
    }

    pub fn run_requested(&self) -> (Self, Option<RunRequest>) {
        if self.is_running() {
            return (self.clone(), None);
        }
        let text = match self.effective_selection() {
            Some(range) => &self.script[range],
            None => self.script.as_str(),
        };
        if text.trim().is_empty() {
            let next = Self {
                notice: Some(ShellNotice::EmptyScript),
                ..self.clone()
            };
            return (next, None);
        }
        let (next, request) = self.started(text);
        (next, Some(request))
    }

    /// Starts a run of exactly `script_to_run` (whatever the editor holds now): the next run id,
    /// output, notice and the previous run's value cleared (the output mode stays). The caller has checked that nothing is running and that the
    /// text is worth running (a confirmed run sends the text as it was when Run was clicked).
    pub fn started(&self, script_to_run: &str) -> (Self, RunRequest) {
        let run_id = self.next_run_id;
        let request = RunRequest {
            run_id,
            script: script_to_run.to_string(),
        };
        let next = Self {
            output: OutputLog::default(),
            dropped_lines: 0,
            status: ShellStatus::Running { run_id },
            next_run_id: run_id + 1,
            notice: None,
            result: ResultView::pending(run_id),
            ..self.clone()
        };
        (next, request)
    }

    pub fn output_arrived(&self, run_id: u64, stream: Stream, text: &str) -> Self {
        if !self.is_current(run_id) || text.is_empty() {
            return self.clone();
        }
        let (output, dropped) = self.output.appended(stream, text);
        Self {
            output,
            dropped_lines: self.dropped_lines + dropped,
            ..self.clone()
        }
    }

    /// Number of retained output lines.
    pub fn line_count(&self) -> usize {
        self.output.len()
    }

    /// The retained output line at `index`, 0 being the oldest.
    pub fn line(&self, index: usize) -> Option<&OutputLine> {
        self.output.get(index)
    }

    pub fn iter_lines(&self) -> impl Iterator<Item = &OutputLine> {
        self.output.iter()
    }

    /// The run ended: a value that arrived stays; without one, a clean exit means the script
    /// returned nothing and anything else that the run failed.
    pub fn finished(&self, run_id: u64, outcome: Outcome) -> Self {
        let clean = outcome == Outcome::Exited(0);
        self.ended(run_id, ShellNotice::Finished(outcome), clean)
    }

    pub fn refused(&self, run_id: u64, reason: ShellRefusal) -> Self {
        self.ended(run_id, ShellNotice::Refused(reason), false)
    }

    pub fn tool_problem(&self, problem: ToolProblem) -> Self {
        Self {
            notice: Some(ShellNotice::ToolProblem(problem)),
            ..self.clone()
        }
    }

    fn ended(&self, run_id: u64, notice: ShellNotice, clean: bool) -> Self {
        if !self.is_current(run_id) {
            return self.clone();
        }
        Self {
            output: self.output.closed(),
            status: ShellStatus::Idle,
            notice: Some(notice),
            result: self.result.ended(clean),
            ..self.clone()
        }
    }

    /// The selection if it is a non-empty range inside the script, widened to char boundaries.
    fn effective_selection(&self) -> Option<Range<usize>> {
        let range = self.selection.as_ref()?;
        let len = self.script.len();
        if range.start >= range.end || range.start >= len {
            return None;
        }
        let mut start = range.start;
        let mut end = range.end.min(len);
        while !self.script.is_char_boundary(start) {
            start -= 1;
        }
        while !self.script.is_char_boundary(end) {
            end += 1;
        }
        Some(start..end)
    }
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
