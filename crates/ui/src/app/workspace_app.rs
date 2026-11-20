//! The workspace file inside the running app (#159).
#![allow(dead_code)] // wired in by the next commits
use super::persistence::PersistGuard;
use super::tabs::TabId;
use super::workspace_file::WorkspaceFile;
use std::path::PathBuf;
use std::time::Instant;

/// Where and when `workspace.json` is written.
#[derive(Debug, Default)]
pub struct WorkspaceSync {
    pub path: Option<PathBuf>,
    pub guard: PersistGuard,
    pub written: Option<WorkspaceFile>,
    pub writes: u64,
    pub(super) pending: Option<(WorkspaceFile, Instant)>,
    pub(super) failed: Option<WorkspaceFile>,
    pub(super) marker: Option<(Option<TabId>, u64, u64)>,
    pub(super) streak: bool,
    pub(super) announced: bool,
}
