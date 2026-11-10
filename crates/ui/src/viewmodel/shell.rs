//! View-model of the Shell tab: the script, its selection, the captured output and the run state.
//! Pure and egui-free; every updater returns a new value and never mutates. No user-facing strings.
use super::shell_format::format_script;
use curlybrace_core::runtime::ShellRefusal;
use curlybrace_core::shell::{Outcome, Stream};
use curlybrace_core::tools::{ServerVersion, ToolError, ToolVersion};
use std::ops::Range;

/// The most output lines kept; older lines are dropped first.
pub const MAX_OUTPUT_LINES: usize = 10_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShellStatus {
    #[default]
    Idle,
    Running {
        run_id: u64,
    },
}

/// One line of output. `open` is true while the line has not yet seen its newline, so the next
/// chunk of the same stream continues it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLine {
    pub stream: Stream,
    pub text: String,
    pub open: bool,
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
    pub output: Vec<OutputLine>,
    pub status: ShellStatus,
    pub next_run_id: u64,
    pub notice: Option<ShellNotice>,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            script: String::new(),
            selection: None,
            output: Vec::new(),
            status: ShellStatus::Idle,
            next_run_id: 1,
            notice: None,
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

    fn is_current(&self, run_id: u64) -> bool {
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
        let run_id = self.next_run_id;
        let request = RunRequest {
            run_id,
            script: text.to_string(),
        };
        let next = Self {
            output: Vec::new(),
            status: ShellStatus::Running { run_id },
            next_run_id: run_id + 1,
            notice: None,
            ..self.clone()
        };
        (next, Some(request))
    }

    pub fn output_arrived(&self, run_id: u64, stream: Stream, text: &str) -> Self {
        if !self.is_current(run_id) || text.is_empty() {
            return self.clone();
        }
        let mut output = self.output.clone();
        for piece in text.split_inclusive('\n') {
            let closed = piece.ends_with('\n');
            let body = piece.strip_suffix('\n').unwrap_or(piece);
            match output.last_mut() {
                Some(last) if last.open && last.stream == stream => {
                    last.text.push_str(body);
                    last.open = !closed;
                }
                _ => output.push(OutputLine {
                    stream,
                    text: body.to_string(),
                    open: !closed,
                }),
            }
        }
        if output.len() > MAX_OUTPUT_LINES {
            output.drain(..output.len() - MAX_OUTPUT_LINES);
        }
        Self {
            output,
            ..self.clone()
        }
    }

    pub fn finished(&self, run_id: u64, outcome: Outcome) -> Self {
        self.ended(run_id, ShellNotice::Finished(outcome))
    }

    pub fn refused(&self, run_id: u64, reason: ShellRefusal) -> Self {
        self.ended(run_id, ShellNotice::Refused(reason))
    }

    pub fn tool_problem(&self, problem: ToolProblem) -> Self {
        Self {
            notice: Some(ShellNotice::ToolProblem(problem)),
            ..self.clone()
        }
    }

    fn ended(&self, run_id: u64, notice: ShellNotice) -> Self {
        if !self.is_current(run_id) {
            return self.clone();
        }
        Self {
            status: ShellStatus::Idle,
            notice: Some(notice),
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
