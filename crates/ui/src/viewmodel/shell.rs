//! View-model of the Shell tab: the script, its selection, the captured output and the run state.
//! Pure and egui-free; every updater returns a new value and never mutates. No user-facing strings.
pub use super::shell_check::SyntaxProblem;
use super::shell_check::check;
use super::shell_format::format_script;
pub use super::shell_guard_state::PendingCheck;
use super::shell_history::HistoryView;
pub use super::shell_output::{MAX_LINE_BYTES, MAX_OUTPUT_LINES, OutputLine, OutputLog};
use super::shell_result::{OutputMode, ResultView};
use curlybrace_core::guard::GuardReport;
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
    /// The text's queries are being planned (as guard request `request`) before the run.
    Checking {
        request: u64,
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
    /// The text to run is visibly broken, so it was not run. The problem's range is bytes of
    /// the whole script (not of the selection that was checked); `ran` is what was checked (the
    /// selection, `None` for the whole script), which Run anyway runs. Both go stale once the text
    /// changes, so a text change drops the notice.
    SyntaxProblem {
        problem: SyntaxProblem,
        ran: Option<Range<usize>>,
    },
    /// The guard planned the text to run and some of its queries would scan a collection (or sort
    /// in memory), so it was not run. `report` holds those findings; `unchecked` counts every
    /// query that could not be judged (unreadable, past the cap, or not answered by the check);
    /// `ran` is what was checked, which Run anyway runs. A text change drops the notice, as it
    /// does a syntax notice.
    IndexMiss {
        report: GuardReport,
        unchecked: usize,
        ran: Option<Range<usize>>,
    },
    /// The check let a production run go on, but the write warning was already taken by
    /// another write, so the run was not started; Run again once that one is answered.
    GuardNotStarted,
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
    /// Bumped each time the editor must take the keyboard focus (Go to problem); the editor
    /// takes it once for every value greater than the last one it saw. Nothing else changes it.
    pub editor_focus: u64,
    /// While `Checking`: what the check is for. `None` in any other status.
    pub pending_check: Option<PendingCheck>,
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
            editor_focus: 0,
            pending_check: None,
        }
    }
}

impl ShellState {
    pub fn is_running(&self) -> bool {
        matches!(self.status, ShellStatus::Running { .. })
    }

    /// Running, or checking its queries before a run: a second Run is refused and Cancel works.
    pub fn is_busy(&self) -> bool {
        matches!(
            self.status,
            ShellStatus::Running { .. } | ShellStatus::Checking { .. }
        )
    }

    /// Whether the Run button is enabled: idle and the script has non-whitespace text.
    pub fn can_run(&self) -> bool {
        !self.is_busy() && !self.script.trim().is_empty()
    }

    pub(super) fn is_current(&self, run_id: u64) -> bool {
        self.status == (ShellStatus::Running { run_id })
    }

    /// The text or the selection changed in the editor. A syntax or index notice goes once the
    /// text changes (its range would point at the wrong bytes), and so does a check in flight
    /// (it checked other text); the other notices stay.
    pub fn edited(&self, script: String, selection: Option<Range<usize>>) -> Self {
        Self {
            selection,
            ..self.text_set(script)
        }
    }

    pub fn formatted(&self) -> Self {
        let result = format_script(&self.script, self.effective_selection());
        Self {
            selection: result.selection,
            ..self.text_set(result.text)
        }
    }

    /// The script becomes `script`: when it changed, the notices about the old text and a check
    /// of it go (the check's late answer then matches nothing).
    fn text_set(&self, script: String) -> Self {
        let next = match script != self.script {
            true => self.check_dropped(),
            false => self.clone(),
        };
        Self {
            notice: self.notice_for(&script),
            script,
            ..next
        }
    }

    /// Run: the text to run is checked first, and a visibly broken one is not run.
    pub fn run_requested(&self) -> (Self, Option<RunRequest>) {
        self.started_from(self.run_target(true))
    }

    /// Run anyway: as `run_requested`, without the syntax check (this once). While the syntax
    /// or index notice is up it runs exactly what was checked (the text cannot have changed: a
    /// text change drops the notice), whatever the caret or the selection did since (Go to
    /// problem moves the caret); without either it runs the selection, else the whole script.
    pub fn run_anyway(&self) -> (Self, Option<RunRequest>) {
        self.started_from(self.run_target(false))
    }

    /// What a Run (`syntax`: checked) or Run anyway would run, as a range of the script (`None`:
    /// all of it), or the shell as it is to show instead: unchanged while busy, else with the
    /// empty-script or syntax notice. Nothing starts.
    pub fn run_target(&self, syntax: bool) -> Result<Option<Range<usize>>, Box<Self>> {
        let range = match (&self.notice, syntax) {
            (Some(ShellNotice::SyntaxProblem { ran, .. }), false)
            | (Some(ShellNotice::IndexMiss { ran, .. }), false) => ran.clone(),
            _ => self.effective_selection(),
        };
        self.prepared(range, syntax)
    }

    /// The text of `range` of the script (`None`, or a range off its char boundaries: all of it).
    pub fn text_of(&self, range: &Option<Range<usize>>) -> &str {
        range
            .as_ref()
            .and_then(|r| self.script.get(r.clone()))
            .unwrap_or(&self.script)
    }

    fn started_from(
        &self,
        target: Result<Option<Range<usize>>, Box<Self>>,
    ) -> (Self, Option<RunRequest>) {
        match target {
            Ok(range) => {
                let (next, request) = self.started(self.text_of(&range));
                (next, Some(request))
            }
            Err(next) => (*next, None),
        }
    }

    /// Go to problem: a collapsed caret at the problem's first byte (kept inside the script, on a
    /// char boundary), so the next key does not replace the bracket and Run runs the whole
    /// script; the editor is asked for the focus, also when the caret was there already. Nothing
    /// else changes; without a syntax notice nothing does.
    pub fn problem_selected(&self) -> Self {
        let Some(ShellNotice::SyntaxProblem { problem, .. }) = &self.notice else {
            return self.clone();
        };
        let caret = self.boundary(problem.range.start);
        Self {
            selection: Some(caret..caret),
            editor_focus: self.editor_focus.saturating_add(1),
            ..self.clone()
        }
    }

    /// `range` of the script (`None`: all of it) when it may run, checked first when `checked`.
    fn prepared(
        &self,
        range: Option<Range<usize>>,
        checked: bool,
    ) -> Result<Option<Range<usize>>, Box<Self>> {
        if self.is_busy() {
            return Err(Box::new(self.clone()));
        }
        // A range off the script's char boundaries cannot happen (a text change drops the notice
        // that stored it); should it, the whole script is the safe reading.
        let range = range.filter(|r| self.script.get(r.clone()).is_some());
        let (start, text) = match range.clone() {
            Some(range) => (range.start, &self.script[range]),
            None => (0, self.script.as_str()),
        };
        if text.trim().is_empty() {
            return Err(Box::new(self.with_notice(ShellNotice::EmptyScript)));
        }
        if let Some(problem) = checked.then(|| check(text)).flatten() {
            let notice = ShellNotice::SyntaxProblem {
                problem: problem.placed_in(&self.script, start),
                ran: range,
            };
            return Err(Box::new(self.with_notice(notice)));
        }
        Ok(range)
    }

    /// A Run refused before it started: only the notice changes, so the previous run's output
    /// and value stay.
    fn with_notice(&self, notice: ShellNotice) -> Self {
        Self {
            notice: Some(notice),
            ..self.clone()
        }
    }

    /// The notice once the script becomes `script`.
    fn notice_for(&self, script: &str) -> Option<ShellNotice> {
        match &self.notice {
            Some(ShellNotice::SyntaxProblem { .. } | ShellNotice::IndexMiss { .. })
                if script != self.script =>
            {
                None
            }
            other => other.clone(),
        }
    }

    /// `at` kept inside the script and moved back to a char boundary.
    fn boundary(&self, at: usize) -> usize {
        let mut at = at.min(self.script.len());
        while !self.script.is_char_boundary(at) {
            at -= 1;
        }
        at
    }

    /// Starts a run of exactly `script_to_run` (whatever the editor holds now): the next run id,
    /// output, notice and the previous run's value cleared (the output mode stays). The caller
    /// has checked that nothing is running and that the text is worth running (a confirmed run
    /// sends the text as it was when Run was clicked).
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
            pending_check: None,
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
