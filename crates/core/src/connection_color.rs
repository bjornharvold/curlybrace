//! The colour tag as stored: a preset this build knows, or a raw value it does not (for example
//! one written by a newer build) that is kept so a save writes it back unchanged.
use serde::{Deserialize, Serialize};

/// A preset colour tag a user can give a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionColor {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
}

impl ConnectionColor {
    pub const ALL: [ConnectionColor; 6] = [
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::Purple,
    ];
}

/// A saved colour. `Known` serialises exactly as the `ConnectionColor` variant name and
/// `Unrecognised` as its raw string, so loading and saving never changes the stored value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum SavedColor {
    Known(ConnectionColor),
    Unrecognised(String),
}

impl SavedColor {
    /// The preset to draw, or `None` for a colour this build does not recognise.
    pub fn known(&self) -> Option<ConnectionColor> {
        match self {
            Self::Known(color) => Some(*color),
            Self::Unrecognised(_) => None,
        }
    }
}

/// Reads the colour leniently so one bad entry never fails loading the whole connection list:
/// a known name is `Known`, any other string is kept as `Unrecognised`, and a missing or null
/// value is `None`. A non-string value has nothing to keep and loads as `None`. Unrecognised
/// and malformed values warn without naming the connection, its URI or the raw value.
pub(crate) fn lenient_color<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<SavedColor>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(raw)) => Some(
            match serde_json::from_value(serde_json::Value::String(raw.clone())) {
                Ok(color) => SavedColor::Known(color),
                Err(_) => {
                    log::warn!(
                        "a saved connection has an unrecognised colour; keeping it, shown as none"
                    );
                    SavedColor::Unrecognised(raw)
                }
            },
        ),
        Some(_) => {
            log::warn!("a saved connection has a malformed colour; loading it without one");
            None
        }
    })
}
