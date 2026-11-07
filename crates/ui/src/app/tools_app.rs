//! The tool paths inside the running app: loading and saving `tools.json`, starting the version
//! checks and the file dialog, and following a connection that is deleted or duplicated. The
//! pure rules are in `tools_state`; this is the part that touches threads and the disk.
use super::connection_ops;
use super::file_picker::PickPoll;
use super::reducer::Input;
use super::tool_picker;
use super::tool_probe::{self, ProbeAnswer};
use super::tool_settings::{self, ToolsSaveError};
use super::tools_state::{ToolsAction, ToolsState};
use super::{ConfigPath, CurlybraceApp, persistence};
use curlybrace_core::tools::ToolKind;

impl CurlybraceApp {
    /// Loads the tool paths from `path`. A file that cannot be used is backed up and the tools
    /// are looked up on `PATH`, with the recovery banner (an earlier startup banner keeps its
    /// place and this one waits behind it).
    pub fn with_tools_path(mut self, path: ConfigPath) -> Self {
        let started = match &path {
            Ok(p) => tool_settings::load(p, &persistence::back_up),
            Err(_) => tool_settings::ToolsStartup {
                settings: Default::default(),
                guard: Default::default(),
                recovery: None,
            },
        };
        self.state.tools = ToolsState::from_settings(started.settings);
        if let Some(recovery) = started.recovery {
            self.state = std::mem::take(&mut self.state).with_recovery_notice(recovery);
        }
        self.tools_path = path;
        self.tools_guard = started.guard;
        self
    }

    /// The `PATH` that `PATH` choices are searched in. Tests pass `None` or a temp folder so
    /// that no program of the developer's machine is ever run.
    pub fn with_tool_path_var(self, path: Option<std::ffi::OsString>) -> Self {
        Self {
            tool_path_var: path,
            ..self
        }
    }

    /// After any step: writes the tool paths if they changed, starts the version checks the
    /// step asked for and opens the file dialog the state is waiting for.
    pub(super) fn tools_changed(&mut self, before: &curlybrace_core::tools::ToolSettings) {
        if self.state.tools.settings != *before {
            self.save_tools();
        }
        self.raise_tools_notice();
        let (tools, requests) = std::mem::take(&mut self.state.tools).take_queue();
        self.state.tools = tools;
        for request in requests {
            tool_probe::spawn(
                request,
                self.probe_sender.clone(),
                self.repaint.clone(),
                self.tool_path_var.clone(),
            );
        }
        self.open_tool_dialog_if_wanted();
    }

    /// Shows the waiting save-failure notice once no other banner is showing.
    fn raise_tools_notice(&mut self) {
        if self.state.banner.is_none() {
            self.state.banner = self.tools_notice.take();
        }
    }

    /// A dialog belongs to a request in the state, which keeps asking until it is answered.
    fn open_tool_dialog_if_wanted(&mut self) {
        if self.tool_pick.is_some() {
            return;
        }
        if let Some(target) = self.state.tools.browsing() {
            self.tool_pick = Some(tool_picker::start(
                self.picker.clone(),
                target,
                self.repaint.clone(),
            ));
        }
    }

    /// Writes the tool paths after a change. A failed write is a banner like a failed save of the
    /// connection list; the change itself stays applied for this run.
    pub(super) fn save_tools(&mut self) {
        let result = match &self.tools_path {
            Ok(path) => tool_settings::saved(path, &self.tools_guard, &self.state.tools.settings),
            Err(e) => Err(ToolsSaveError::Persist(e.clone())),
        };
        if let Err(e) = result {
            log::warn!("could not save the tool paths: {e:?}");
            // A banner that is already showing keeps its place; the notice waits for the slot
            // (`raise_tools_notice`) so the failure is never silent.
            self.tools_notice = Some(e.banner());
            self.raise_tools_notice();
        }
    }

    /// Hands the answers of finished version checks to the reducer.
    pub(super) fn drain_probe_answers(&mut self) {
        let answers: Vec<ProbeAnswer> = self.probe_answers.try_iter().collect();
        for answer in answers {
            self.dispatch(Input::Tools(ToolsAction::Probed {
                scope: answer.scope,
                kind: answer.kind,
                request: answer.request,
                result: answer.result,
            }));
        }
    }

    /// A delete that reached the disk takes the overrides of the connections it removed along,
    /// under the same rule as the history purge.
    pub(super) fn forget_tools_of_removed(&mut self, saved: bool, before: &[uuid::Uuid]) {
        if !saved {
            return;
        }
        let removed: Vec<uuid::Uuid> = before
            .iter()
            .filter(|id| self.state.connection(**id).is_none())
            .copied()
            .collect();
        for id in removed {
            self.state.tools = std::mem::take(&mut self.state.tools).connection_removed(id);
        }
    }

    /// A duplicate starts with the overrides of the connection it was copied from.
    pub(super) fn copy_tools_to_new(&mut self, source: uuid::Uuid, before: &[uuid::Uuid]) {
        let added: Vec<uuid::Uuid> = connection_ops::saved_ids(&self.state)
            .into_iter()
            .filter(|id| !before.contains(id))
            .collect();
        for id in added {
            self.state.tools =
                std::mem::take(&mut self.state.tools).connection_duplicated(source, id);
        }
    }

    /// Opens the tool paths dialog with the runnable files found in the usual install folders.
    /// Only looks at files; nothing is run until the person picks one.
    pub(super) fn open_tool_settings(&mut self) {
        let suggestions = ToolKind::ALL
            .into_iter()
            .map(|kind| (kind, curlybrace_core::tools::suggestions(kind)))
            .filter(|(_, found)| !found.is_empty())
            .collect();
        self.dispatch(Input::Tools(ToolsAction::OpenSettings { suggestions }));
    }

    /// Hands the answer of the executable dialog to the reducer once it has come.
    pub(super) fn poll_tool_picker(&mut self) {
        let Some(PickPoll::Done(path)) = self.tool_pick.as_ref().map(|p| p.poll()) else {
            return;
        };
        let opened_for = self.tool_pick.take().map(|pick| pick.target());
        if opened_for == self.state.tools.browsing() {
            self.dispatch(Input::Tools(ToolsAction::Browsed { path }));
        } else {
            // The file was chosen for something that is no longer asked about (its connection
            // was deleted, say). Dropping it, and opening the dialog the state now wants.
            self.open_tool_dialog_if_wanted();
        }
    }
}
