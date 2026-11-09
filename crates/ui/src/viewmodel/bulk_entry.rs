//! Whether the documents toolbar may start a bulk write, and the reason when it may not. The
//! reducer's `Open` and the toolbar both read it from the state, so the buttons are enabled
//! exactly when opening would start a flow.
use super::connection_tag::READ_ONLY_REASON_KEY;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BulkEntry {
    /// A flow can be opened.
    #[default]
    Available,
    /// The active session is read-only.
    ReadOnly,
    /// The displayed page holds partial documents (a projection).
    Projected,
    /// Another bulk flow is open, or a write is parked or in flight.
    Busy,
}

impl BulkEntry {
    pub fn is_available(self) -> bool {
        self == Self::Available
    }

    /// The i18n key of the reason the buttons are off, if they are.
    pub fn reason_key(self) -> Option<&'static str> {
        match self {
            Self::Available => None,
            Self::ReadOnly => Some(READ_ONLY_REASON_KEY),
            Self::Projected => Some("bulk.projected_reason"),
            Self::Busy => Some("bulk.busy_reason"),
        }
    }
}
