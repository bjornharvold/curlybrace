//! Which way the edit dialog was opened, for its heading.

/// How the open edit dialog got its form. `Edit` is the default and what closing resets to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditMode {
    /// A saved connection is being edited.
    #[default]
    Edit,
    /// A blank form for a new connection.
    New,
    /// A copy of an existing connection. The copy is already saved (to the config file and the
    /// keychain) when the dialog opens, so Cancel keeps it.
    Duplicate,
}

impl EditMode {
    /// The i18n key of the dialog heading.
    pub const fn title_key(self) -> &'static str {
        match self {
            Self::Edit => "connections.edit_title",
            Self::New => "connections.new_title",
            Self::Duplicate => "connections.duplicate_title",
        }
    }
}
