//! What one tool's row in the settings shows: the choice that applies, whether it is the
//! connection's own, and the line that says how the check went. No egui here.
use crate::I18n;
use crate::app::tools_state::{ProbeScope, ProbeStatus, ToolsState};
use curlybrace_core::tools::{
    Compatibility, ServerVersion, ToolChoice, ToolError, ToolKind, ToolVersion, compatibility,
};

/// How the check of a tool stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    Unchecked,
    Checking,
    Found { version: ToolVersion, too_old: bool },
    Problem(ToolError),
}

/// One tool in one scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolRow {
    pub kind: ToolKind,
    /// The choice that applies: the connection's own, else the global one.
    pub choice: ToolChoice,
    /// A connection row follows the global settings (`false`) or has its own choice (`true`).
    /// The global row is always its own.
    pub own: bool,
    pub status: ToolStatus,
}

/// The row for `kind` in `scope`. `server` is the version of the server the tool would work
/// against, when one is known; without it a version is never called too old.
pub fn tool_row(
    tools: &ToolsState,
    scope: ProbeScope,
    kind: ToolKind,
    server: Option<ServerVersion>,
) -> ToolRow {
    let (choice, own) = match scope {
        ProbeScope::Global => (tools.settings.global(kind), true),
        ProbeScope::Connection(id) => (
            tools.settings.effective(kind, Some(id)),
            tools.settings.overrides(id, kind),
        ),
    };
    let status = match tools.shown_status(scope, kind) {
        None => ToolStatus::Unchecked,
        Some(ProbeStatus::Checking { .. }) => ToolStatus::Checking,
        Some(ProbeStatus::Done(Ok(version))) => ToolStatus::Found {
            version: *version,
            too_old: compatibility(kind, *version, server) == Compatibility::TooOld,
        },
        Some(ProbeStatus::Done(Err(error))) => ToolStatus::Problem(*error),
    };
    ToolRow {
        kind,
        choice,
        own,
        status,
    }
}

/// How serious a status line is, for the colour and the order a person reads it in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusLine {
    pub text: String,
    pub tone: Tone,
}

/// The sentence under a tool. A too-old version names the server it falls short of.
pub fn status_line(status: &ToolStatus, server: Option<ServerVersion>, i18n: &I18n) -> StatusLine {
    let (text, tone) = match status {
        ToolStatus::Unchecked => (i18n.t("tools.not_checked"), Tone::Neutral),
        ToolStatus::Checking => (i18n.t("tools.checking"), Tone::Neutral),
        ToolStatus::Found { version, too_old } => {
            let shown = version_text(*version);
            match (too_old, server) {
                (true, Some(server)) => (
                    i18n.t_with(
                        "tools.too_old",
                        &[
                            ("version", &shown),
                            ("server", &format!("{}.{}", server.major, server.minor)),
                        ],
                    ),
                    Tone::Warning,
                ),
                _ => (
                    i18n.t_with("tools.version", &[("version", &shown)]),
                    Tone::Neutral,
                ),
            }
        }
        ToolStatus::Problem(error) => (i18n.t(error.i18n_key()), Tone::Error),
    };
    StatusLine { text, tone }
}

pub(crate) fn version_text(version: ToolVersion) -> String {
    format!("{}.{}.{}", version.major, version.minor, version.patch)
}

#[cfg(test)]
#[path = "tool_rows_tests.rs"]
mod tests;
