//! The Shell tabs of the open sessions and the bookkeeping that keeps their run ids apart.
//! Pure values: every change returns a new one.
use crate::viewmodel::shell::{ShellState, ShellStatus};
use curlybrace_core::tools::ServerVersion;
use std::collections::BTreeMap;
use uuid::Uuid;

/// Not part of a session view: keyed by session, created when first used, reset to idle when its
/// session's connection is lost or reopened, and dropped when the session is closed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShellTabs {
    tabs: BTreeMap<Uuid, ShellState>,
    /// The first run id a newly created shell may use: past every id a dropped shell handed out,
    /// so a late event of a closed session's run never matches a run of the reopened one.
    run_floor: u64,
    /// The server version of an open session, when known. Nothing reports it yet (`Connected`
    /// carries none), so it stays empty and the too-old check stays silent.
    server_versions: BTreeMap<Uuid, ServerVersion>,
}

impl ShellTabs {
    /// The shell of `session`, if it was ever used.
    pub fn get(&self, session: Uuid) -> Option<&ShellState> {
        self.tabs.get(&session)
    }

    /// The shell of `session`, or a fresh one whose run ids start past every dropped shell's.
    pub fn get_or_new(&self, session: Uuid) -> ShellState {
        self.tabs
            .get(&session)
            .cloned()
            .unwrap_or_else(|| ShellState {
                next_run_id: self.run_floor.max(1),
                ..ShellState::default()
            })
    }

    pub fn with(self, session: Uuid, shell: ShellState) -> Self {
        let mut tabs = self.tabs;
        tabs.insert(session, shell);
        Self { tabs, ..self }
    }

    pub fn server_version(&self, session: Uuid) -> Option<ServerVersion> {
        self.server_versions.get(&session).copied()
    }

    pub fn with_server_version(self, session: Uuid, version: ServerVersion) -> Self {
        let mut server_versions = self.server_versions;
        server_versions.insert(session, version);
        Self {
            server_versions,
            ..self
        }
    }

    /// The connection of `session` was lost or is being reopened: its shell goes back to idle (a
    /// guard check in flight is forgotten, and its late answer matches nothing) and what was known of its server is forgotten. The script and the run counter stay (a
    /// restarted counter would let a late event of the old run match a new one); the output
    /// stays until the next run clears it, and so does the run's value (a run cut short here
    /// has none: the Table and Tree say it failed). The chosen output mode stays.
    pub fn reset(self, session: Uuid) -> Self {
        let mut server_versions = self.server_versions;
        server_versions.remove(&session);
        let next = Self {
            server_versions,
            ..self
        };
        let Some(shell) = next.tabs.get(&session) else {
            return next;
        };
        let idle = ShellState {
            status: ShellStatus::Idle,
            pending_check: None,
            output: shell.output.closed(),
            notice: None,
            ..shell.result_interrupted()
        };
        next.with(session, idle)
    }

    /// The session was closed: its shell and server version go, and the run ids it handed out
    /// are never handed out again.
    pub fn dropped(self, session: Uuid) -> Self {
        let run_floor = self
            .tabs
            .get(&session)
            .map_or(self.run_floor, |s| s.next_run_id.max(self.run_floor));
        let mut tabs = self.tabs;
        tabs.remove(&session);
        let mut server_versions = self.server_versions;
        server_versions.remove(&session);
        Self {
            tabs,
            run_floor,
            server_versions,
        }
    }
}
