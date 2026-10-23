//! The colour tag as stored: a preset this build knows, or a raw value it does not (for example
//! one written by a newer build) that is kept so a save writes it back unchanged.
use serde::de::value::{Error as ValueError, StrDeserializer};
use serde::de::{Error as _, IntoDeserializer};
use serde::{Deserialize, Deserializer, Serialize};

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

impl ConnectionColor {
    /// The preset named exactly `name` (its variant name), if any. Borrows the string and
    /// reuses the derived `Deserialize`, so the accepted names cannot drift from the enum.
    fn from_name(name: &str) -> Option<Self> {
        let de: StrDeserializer<'_, ValueError> = name.into_deserializer();
        Self::deserialize(de).ok()
    }
}

/// A saved colour. `Known` serialises exactly as the `ConnectionColor` variant name and
/// `Unrecognised` as the raw JSON value that was loaded (any non-null value that is not a
/// known name: for example one written by a newer build), so loading and saving never changes
/// the stored value. A null or missing colour is `None` on the config, not a `SavedColor`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum SavedColor {
    Known(ConnectionColor),
    Unrecognised(serde_json::Value),
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

/// Reads any non-null value without failing, so one odd entry never fails loading the whole
/// connection list. A value that is not a known colour name is kept as `Unrecognised` and
/// warns, without naming the connection, its URI or the raw value.
impl<'de> Deserialize<'de> for SavedColor {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.is_null() {
            return Err(D::Error::custom("a saved colour cannot be null"));
        }
        if let Some(color) = value.as_str().and_then(ConnectionColor::from_name) {
            return Ok(Self::Known(color));
        }
        log::warn!("a saved connection has an unrecognised colour; keeping it, shown as none");
        Ok(Self::Unrecognised(value))
    }
}
