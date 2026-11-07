//! The tool paths and the version checks they ask for.
//!
//! Pure state: a choice changes the settings and queues a check; the app layer takes the queue,
//! runs the checks on worker threads and feeds the answers back through [`ToolsState::probed`].
//! Every check carries a number and only the newest one for a tool is wanted, so the answer to a
//! choice that was replaced meanwhile is dropped (the same rule the runtime applies to epochs).
use curlybrace_core::tools::{ToolChoice, ToolError, ToolKind, ToolSettings, ToolVersion};
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

/// Whose choice a check is for: the global default or one connection's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProbeScope {
    Global,
    Connection(Uuid),
}

/// Where a check stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeStatus {
    Checking { request: u64 },
    Done(Result<ToolVersion, ToolError>),
}

/// A check waiting to be started. `choice` is what to resolve and run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeRequest {
    pub scope: ProbeScope,
    pub kind: ToolKind,
    pub request: u64,
    pub choice: ToolChoice,
}

/// What the user or a finished check does to the tool paths.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolsAction {
    ChooseGlobal {
        kind: ToolKind,
        choice: ToolChoice,
    },
    ChooseOverride {
        connection: Uuid,
        kind: ToolKind,
        choice: ToolChoice,
    },
    /// The connection follows the global default again.
    UseGlobal {
        connection: Uuid,
        kind: ToolKind,
    },
    Recheck {
        scope: ProbeScope,
        kind: ToolKind,
    },
    /// Open the global settings dialog; `suggestions` are the runnable files found in the common
    /// folders (looked up by the caller, which is the one that reads the disk).
    OpenSettings {
        suggestions: BTreeMap<ToolKind, Vec<PathBuf>>,
    },
    CloseSettings,
    /// Ask for an executable with the system file dialog.
    Browse {
        scope: ProbeScope,
        kind: ToolKind,
    },
    /// The file dialog answered; `None` when it was cancelled.
    Browsed {
        path: Option<PathBuf>,
    },
    /// The answer of check number `request`.
    Probed {
        scope: ProbeScope,
        kind: ToolKind,
        request: u64,
        result: Result<ToolVersion, ToolError>,
    },
}

/// The open global settings dialog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolsDialog {
    suggestions: BTreeMap<ToolKind, Vec<PathBuf>>,
}

impl ToolsDialog {
    /// Runnable files for `kind` found where package managers install them.
    pub fn suggestions(&self, kind: ToolKind) -> &[PathBuf] {
        self.suggestions.get(&kind).map_or(&[], Vec::as_slice)
    }
}

type Key = (ProbeScope, ToolKind);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolsState {
    pub settings: ToolSettings,
    probes: BTreeMap<Key, ProbeStatus>,
    queue: Vec<ProbeRequest>,
    issued: u64,
    dialog: Option<ToolsDialog>,
    browsing: Option<(ProbeScope, ToolKind)>,
    /// Moves every time the settings may have changed, so callers can tell whether the file
    /// needs writing with a number compare instead of comparing (or cloning) the settings.
    revision: u64,
}

impl ToolsState {
    /// The settings read at launch; nothing has been checked yet.
    pub fn from_settings(settings: ToolSettings) -> Self {
        Self {
            settings,
            ..Self::default()
        }
    }

    pub fn status(&self, scope: ProbeScope, kind: ToolKind) -> Option<&ProbeStatus> {
        self.probes.get(&(scope, kind))
    }

    /// See the field: changes whenever the settings may have.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether checks are waiting to be started.
    pub fn has_requests(&self) -> bool {
        !self.queue.is_empty()
    }

    pub fn dialog(&self) -> Option<&ToolsDialog> {
        self.dialog.as_ref()
    }

    pub fn browsing(&self) -> Option<(ProbeScope, ToolKind)> {
        self.browsing
    }

    /// Opens the global settings dialog and checks every global tool, so the versions shown are
    /// current.
    pub fn open_settings(self, suggestions: BTreeMap<ToolKind, Vec<PathBuf>>) -> Self {
        let opened = Self {
            dialog: Some(ToolsDialog { suggestions }),
            ..self
        };
        ToolKind::ALL.into_iter().fold(opened, |state, kind| {
            state.checked(ProbeScope::Global, kind)
        })
    }

    pub fn close_settings(self) -> Self {
        Self {
            dialog: None,
            ..self
        }
    }

    /// The system file dialog is wanted for this tool. Only one is open at a time.
    pub fn begin_browse(self, scope: ProbeScope, kind: ToolKind) -> Self {
        match self.browsing {
            Some(_) => self,
            None => Self {
                browsing: Some((scope, kind)),
                ..self
            },
        }
    }

    /// The file dialog answered: a file becomes the choice for what it was opened for.
    pub fn browsed(self, path: Option<PathBuf>) -> Self {
        let Some((scope, kind)) = self.browsing else {
            return self;
        };
        let done = Self {
            browsing: None,
            ..self
        };
        match (path.map(ToolChoice::Executable), scope) {
            (None, _) => done,
            (Some(choice), ProbeScope::Global) => done.choose_global(kind, choice),
            (Some(choice), ProbeScope::Connection(id)) => done.choose_override(id, kind, choice),
        }
    }

    /// Sets the global default for `kind` and checks it.
    pub fn choose_global(self, kind: ToolKind, choice: ToolChoice) -> Self {
        let settings = self.settings.with_global(kind, choice);
        self.with_settings(settings)
            .checked(ProbeScope::Global, kind)
    }

    /// Gives the connection its own choice for `kind` and checks it.
    pub fn choose_override(self, connection: Uuid, kind: ToolKind, choice: ToolChoice) -> Self {
        let settings = self.settings.with_override(connection, kind, choice);
        self.with_settings(settings)
            .checked(ProbeScope::Connection(connection), kind)
    }

    /// The connection follows the global default again; its own check is forgotten.
    pub fn use_global(self, connection: Uuid, kind: ToolKind) -> Self {
        let settings = self.settings.without_override(connection, kind);
        self.with_settings(settings)
            .forgotten(ProbeScope::Connection(connection), kind)
    }

    /// Checks again whatever applies to `scope` now. A connection that follows the global choice
    /// has no check of its own: it shows the global one, so that is the one repeated.
    pub fn recheck(self, scope: ProbeScope, kind: ToolKind) -> Self {
        let scope = self.effective_scope(scope, kind);
        self.checked(scope, kind)
    }

    /// The check shown for a tool in a scope: the connection's own when it has its own file,
    /// else the global one.
    pub fn shown_status(&self, scope: ProbeScope, kind: ToolKind) -> Option<&ProbeStatus> {
        self.status(self.effective_scope(scope, kind), kind)
    }

    /// The scope whose check applies: a connection without its own file uses the global one.
    fn effective_scope(&self, scope: ProbeScope, kind: ToolKind) -> ProbeScope {
        match scope {
            ProbeScope::Connection(id) if !self.settings.overrides(id, kind) => ProbeScope::Global,
            other => other,
        }
    }

    /// A deleted connection: its overrides, checks and waiting requests go.
    pub fn connection_removed(self, connection: Uuid) -> Self {
        let scope = ProbeScope::Connection(connection);
        let settings = self.settings.without_connection(connection);
        let revision = self.revision_after(&settings);
        Self {
            settings,
            revision,
            probes: self
                .probes
                .into_iter()
                .filter(|((s, _), _)| *s != scope)
                .collect(),
            queue: self
                .queue
                .into_iter()
                .filter(|r| r.scope != scope)
                .collect(),
            browsing: self.browsing.filter(|(s, _)| *s != scope),
            ..self
        }
    }

    /// A duplicated connection starts with the same overrides, not yet checked.
    pub fn connection_duplicated(self, from: Uuid, to: Uuid) -> Self {
        let settings = self.settings.with_connection_copied(from, to);
        self.with_settings(settings)
    }

    /// The answer to check number `request`. Kept only while that is still the newest check for
    /// the tool; otherwise nothing changes.
    pub fn probed(
        self,
        scope: ProbeScope,
        kind: ToolKind,
        request: u64,
        result: Result<ToolVersion, ToolError>,
    ) -> Self {
        let wanted = matches!(
            self.probes.get(&(scope, kind)),
            Some(ProbeStatus::Checking { request: current }) if *current == request
        );
        if !wanted {
            return self;
        }
        Self {
            probes: with_entry(self.probes, (scope, kind), ProbeStatus::Done(result)),
            ..self
        }
    }

    /// The checks to start now; the queue is empty afterwards.
    pub fn take_queue(mut self) -> (Self, Vec<ProbeRequest>) {
        let queue = std::mem::take(&mut self.queue);
        (self, queue)
    }

    fn with_settings(self, settings: ToolSettings) -> Self {
        let revision = self.revision_after(&settings);
        Self {
            settings,
            revision,
            ..self
        }
    }

    /// The revision for `settings` replacing the current ones: moved only if they differ, so a
    /// choice that changes nothing (or a deleted connection with no overrides) writes no file.
    fn revision_after(&self, settings: &ToolSettings) -> u64 {
        self.revision + u64::from(*settings != self.settings)
    }

    /// Queues a check of what applies to `scope`, replacing one still waiting for the same tool.
    fn checked(self, scope: ProbeScope, kind: ToolKind) -> Self {
        let choice = match scope {
            ProbeScope::Global => self.settings.global(kind),
            ProbeScope::Connection(id) => self.settings.effective(kind, Some(id)),
        };
        let request = self.issued + 1;
        let mut queue: Vec<ProbeRequest> = self
            .queue
            .into_iter()
            .filter(|r| (r.scope, r.kind) != (scope, kind))
            .collect();
        queue.push(ProbeRequest {
            scope,
            kind,
            request,
            choice,
        });
        Self {
            probes: with_entry(
                self.probes,
                (scope, kind),
                ProbeStatus::Checking { request },
            ),
            queue,
            issued: request,
            ..self
        }
    }

    /// Drops the check and any waiting request for the tool.
    fn forgotten(self, scope: ProbeScope, kind: ToolKind) -> Self {
        let mut probes = self.probes;
        probes.remove(&(scope, kind));
        Self {
            probes,
            queue: self
                .queue
                .into_iter()
                .filter(|r| (r.scope, r.kind) != (scope, kind))
                .collect(),
            ..self
        }
    }
}

fn with_entry(
    mut probes: BTreeMap<Key, ProbeStatus>,
    key: Key,
    status: ProbeStatus,
) -> BTreeMap<Key, ProbeStatus> {
    probes.insert(key, status);
    probes
}
