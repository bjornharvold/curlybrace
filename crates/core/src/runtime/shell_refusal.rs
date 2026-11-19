//! Why a shell run was refused.

/// Why a [`super::Command::ShellRun`] was not started.
#[derive(Clone, Debug, PartialEq)]
pub enum ShellRefusal {
    ReadOnly,
    NotConnected,
    AlreadyRunning,
    Invalid(crate::shell::ShellError),
}
