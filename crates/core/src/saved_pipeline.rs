//! Named aggregation pipelines saved per connection.
//!
//! Pure data: every operation returns a new store and leaves the original alone, like
//! `tools::ToolSettings`. A pipeline is kept as the text of each stage, so what comes back is
//! exactly what was typed. Pipelines can hold values from the user's data: nothing here logs
//! a name or a stage, only [`SavedError::error_variant`] is meant for logs.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// The only file format this build reads and writes.
pub const VERSION: u32 = 1;
/// Longest name, in characters, after trimming.
pub const MAX_NAME_CHARS: usize = 80;
/// Most pipelines one connection may keep.
pub const MAX_PER_CONNECTION: usize = 200;
/// Most stage text, in bytes, one pipeline may hold (1 MB).
pub const MAX_STAGE_BYTES: usize = 1_000_000;

/// One stage as typed, and whether it takes part in the pipeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedStage {
    pub text: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedPipeline {
    pub id: Uuid,
    pub name: String,
    /// The `db.collection` it was saved from: a hint for the list, never enforced.
    #[serde(default)]
    pub ns_hint: Option<String>,
    pub saved_unix: i64,
    pub stages: Vec<SavedStage>,
}

/// Why a change to the saved pipelines was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavedError {
    EmptyName,
    /// A name with a control character (a newline, a tab inside, NUL).
    InvalidName,
    NameTooLong,
    NameTaken,
    TooMany,
    TooLarge,
    /// A file from a newer version of the format.
    Unsupported,
}

impl SavedError {
    /// A stable, content-free name for logs and for mapping to a message.
    pub fn error_variant(&self) -> &'static str {
        match self {
            Self::EmptyName => "empty_name",
            Self::InvalidName => "invalid_name",
            Self::NameTooLong => "name_too_long",
            Self::NameTaken => "name_taken",
            Self::TooMany => "too_many",
            Self::TooLarge => "too_large",
            Self::Unsupported => "unsupported",
        }
    }
}

/// The name without surrounding whitespace, when it is 1 to 80 characters with no control
/// character (a newline or NUL would break the list and the dialogs).
pub fn validate_name(name: &str) -> Result<String, SavedError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(SavedError::EmptyName);
    }
    if trimmed.chars().count() > MAX_NAME_CHARS {
        return Err(SavedError::NameTooLong);
    }
    if trimmed.chars().any(char::is_control) {
        return Err(SavedError::InvalidName);
    }
    Ok(trimmed.to_string())
}

/// Whether two names are the same pipeline name: equal ignoring case and surrounding spaces.
pub fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default = "first_version")]
    version: u32,
    #[serde(default)]
    connections: BTreeMap<Uuid, Vec<SavedPipeline>>,
}

fn first_version() -> u32 {
    VERSION
}

impl TryFrom<Raw> for SavedPipelines {
    type Error = String;

    fn try_from(raw: Raw) -> Result<Self, String> {
        if raw.version > VERSION {
            return Err(SavedError::Unsupported.error_variant().to_string());
        }
        let connections = raw
            .connections
            .into_iter()
            .map(|(id, list)| checked_list(list).map(|list| (id, list)))
            .collect::<Result<_, SavedError>>()
            .map_err(|e| e.error_variant().to_string())?;
        Ok(Self {
            version: VERSION,
            connections,
        })
    }
}

/// One connection's list as read from a file. The limits the API enforces hold here too, and a
/// list that breaks one is refused whole, never trimmed: dropping entries on load would lose
/// someone's pipelines silently, while a refusal makes the app back the file up and say so.
/// Only harmless tidying is done: names are trimmed. Refused: a name that is empty, too long or
/// has a control character, a repeated id, a name repeated ignoring case, more than
/// [`MAX_PER_CONNECTION`] entries, more than [`MAX_STAGE_BYTES`] of stage text in one entry.
fn checked_list(list: Vec<SavedPipeline>) -> Result<Vec<SavedPipeline>, SavedError> {
    if list.len() > MAX_PER_CONNECTION {
        return Err(SavedError::TooMany);
    }
    let list = list
        .into_iter()
        .map(|p| {
            let name = validate_name(&p.name)?;
            match p.stages.iter().map(|s| s.text.len()).sum::<usize>() > MAX_STAGE_BYTES {
                true => Err(SavedError::TooLarge),
                false => Ok(SavedPipeline { name, ..p }),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for p in &list {
        if !ids.insert(p.id) || !names.insert(p.name.to_lowercase()) {
            return Err(SavedError::NameTaken);
        }
    }
    Ok(list)
}

/// Every connection's saved pipelines, newest first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Raw")]
pub struct SavedPipelines {
    version: u32,
    connections: BTreeMap<Uuid, Vec<SavedPipeline>>,
}

impl Default for SavedPipelines {
    fn default() -> Self {
        Self {
            version: VERSION,
            connections: BTreeMap::new(),
        }
    }
}

impl SavedPipelines {
    /// The connection's pipelines, most recently saved first.
    pub fn list(&self, connection: Uuid) -> &[SavedPipeline] {
        self.connections.get(&connection).map_or(&[], Vec::as_slice)
    }

    pub fn find(&self, connection: Uuid, id: Uuid) -> Option<&SavedPipeline> {
        self.list(connection).iter().find(|p| p.id == id)
    }

    /// The pipeline whose name matches `name` the way uniqueness does.
    pub fn find_by_name(&self, connection: Uuid, name: &str) -> Option<&SavedPipeline> {
        self.list(connection)
            .iter()
            .find(|p| same_name(&p.name, name))
    }

    /// Whether another pipeline of the connection (not `except`) already has this name,
    /// ignoring case and surrounding spaces.
    pub fn name_taken(&self, connection: Uuid, name: &str, except: Option<Uuid>) -> bool {
        self.list(connection)
            .iter()
            .any(|p| Some(p.id) != except && same_name(&p.name, name))
    }

    /// Saves `stages` under `name`. When the name is taken, `replace` overwrites that pipeline
    /// (same id and name as stored, new content, moved to the front); without it the name is
    /// refused.
    pub fn saved(
        &self,
        connection: Uuid,
        name: &str,
        ns_hint: Option<String>,
        stages: Vec<SavedStage>,
        now_unix: i64,
        replace: bool,
    ) -> Result<Self, SavedError> {
        let name = validate_name(name)?;
        if stages.iter().map(|s| s.text.len()).sum::<usize>() > MAX_STAGE_BYTES {
            return Err(SavedError::TooLarge);
        }
        let found = self.find_by_name(connection, &name);
        let existing = found.map(|p| p.id);
        let name = found.map_or(name, |p| p.name.clone());
        if existing.is_some() && !replace {
            return Err(SavedError::NameTaken);
        }
        if existing.is_none() && self.list(connection).len() >= MAX_PER_CONNECTION {
            return Err(SavedError::TooMany);
        }
        let entry = SavedPipeline {
            id: existing.unwrap_or_else(Uuid::new_v4),
            name,
            ns_hint,
            saved_unix: now_unix,
            stages,
        };
        let rest = self
            .list(connection)
            .iter()
            .filter(|p| Some(p.id) != existing)
            .cloned();
        let list: Vec<SavedPipeline> = std::iter::once(entry).chain(rest).collect();
        Ok(self.with_list(connection, list))
    }

    /// Gives a pipeline a new name, keeping its place and content. An unknown id changes
    /// nothing.
    pub fn renamed(&self, connection: Uuid, id: Uuid, name: &str) -> Result<Self, SavedError> {
        let name = validate_name(name)?;
        if self.name_taken(connection, &name, Some(id)) {
            return Err(SavedError::NameTaken);
        }
        let list = self
            .list(connection)
            .iter()
            .map(|p| {
                if p.id == id {
                    SavedPipeline {
                        name: name.clone(),
                        ..p.clone()
                    }
                } else {
                    p.clone()
                }
            })
            .collect();
        Ok(self.with_list(connection, list))
    }

    /// Removes one pipeline. An unknown id or connection changes nothing.
    pub fn deleted(&self, connection: Uuid, id: Uuid) -> Self {
        let list = self
            .list(connection)
            .iter()
            .filter(|p| p.id != id)
            .cloned()
            .collect();
        self.with_list(connection, list)
    }

    /// For a deleted connection.
    pub fn without_connection(&self, connection: Uuid) -> Self {
        self.with_list(connection, Vec::new())
    }

    /// For a duplicated connection: `to` starts with the pipelines of `from` (same ids, which
    /// only matter within one connection). Unchanged when `from` has none.
    pub fn with_connection_copied(&self, from: Uuid, to: Uuid) -> Self {
        self.with_list(to, self.list(from).to_vec())
    }

    /// The store with this connection's list replaced; an empty list drops the entry.
    fn with_list(&self, connection: Uuid, list: Vec<SavedPipeline>) -> Self {
        let mut connections = self.connections.clone();
        if list.is_empty() {
            connections.remove(&connection);
        } else {
            connections.insert(connection, list);
        }
        Self {
            connections,
            ..self.clone()
        }
    }
}

#[cfg(test)]
#[path = "saved_pipeline_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "saved_pipeline_load_tests.rs"]
mod load_tests;
