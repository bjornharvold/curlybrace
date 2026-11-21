//! Query history and favourite queries (#72).
//!
//! Pure data: every operation returns a new store and leaves the original alone, like
//! `saved_pipeline`. A query is kept as the text of each find-bar field plus the options it
//! applies, so what comes back is exactly what was typed. Queries can hold personal values: no
//! `Debug` here prints text, a name or a namespace, and only [`SavedError::error_variant`] is
//! meant for logs.
use crate::pipeline_options::PipelineOptions;
pub use crate::saved_pipeline::SavedError;
use crate::saved_pipeline::{MAX_NAME_CHARS as NAME_LIMIT, same_name, validate_name};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

/// The only file format this build reads and writes.
pub const VERSION: u32 = 1;
/// Longest favourite name, in characters (the rule of saved pipelines).
pub const MAX_NAME_CHARS: usize = NAME_LIMIT;
/// Most history entries kept for one connection and namespace.
pub const MAX_HISTORY_PER_NAMESPACE: usize = 100;
/// Most favourites one connection may keep.
pub const MAX_FAVOURITES_PER_CONNECTION: usize = 100;
/// Most text, in bytes, one query may hold (256 KiB); a longer one is not recorded.
pub const MAX_ENTRY_BYTES: usize = 256 * 1024;

/// The find bar as typed: each field's text and the Query options it applies.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FindText {
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub sort: String,
    #[serde(default)]
    pub projection: String,
    #[serde(default)]
    pub skip: String,
    #[serde(default)]
    pub limit: String,
    #[serde(default, skip_serializing_if = "PipelineOptions::is_empty")]
    pub options: PipelineOptions,
}

impl FindText {
    /// The text this query holds, in bytes (the options by their serialised size).
    pub fn bytes(&self) -> usize {
        let text: usize = [
            &self.filter,
            &self.sort,
            &self.projection,
            &self.skip,
            &self.limit,
        ]
        .iter()
        .map(|t| t.len())
        .sum();
        let options = match self.options.is_empty() {
            true => 0,
            false => serde_json::to_string(&self.options).map_or(0, |s| s.len()),
        };
        text + options
    }

    /// Nothing typed and no option set: a bar that finds everything.
    pub fn is_blank(&self) -> bool {
        [
            &self.filter,
            &self.sort,
            &self.projection,
            &self.skip,
            &self.limit,
        ]
        .iter()
        .all(|t| t.trim().is_empty())
            && self.options.is_empty()
    }
}

/// Only sizes: the text is the person's data and a debug print ends up in logs and panics.
impl std::fmt::Debug for FindText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FindText")
            .field("bytes", &self.bytes())
            .finish()
    }
}

/// One query that was run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub text: FindText,
    pub ran_unix: i64,
}

/// A named copy of a query for one connection and namespace.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Favourite {
    pub id: Uuid,
    pub name: String,
    pub db: String,
    pub collection: String,
    pub text: FindText,
    pub saved_unix: i64,
}

impl std::fmt::Debug for Favourite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Favourite").field("id", &self.id).finish()
    }
}

type NsKey = (Uuid, String, String);

#[derive(Serialize, Deserialize)]
struct HistoryGroup {
    connection: Uuid,
    db: String,
    collection: String,
    entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize)]
struct FavouriteGroup {
    connection: Uuid,
    items: Vec<Favourite>,
}

/// The file as it is written: lists, because a JSON object cannot be keyed by a tuple.
#[derive(Serialize, Deserialize)]
struct Wire {
    #[serde(default = "first_version")]
    version: u32,
    #[serde(default)]
    history: Vec<HistoryGroup>,
    #[serde(default)]
    favourites: Vec<FavouriteGroup>,
    /// The "Remember queries" switch; a file without it remembers.
    #[serde(default = "remembering_by_default")]
    remember: bool,
}

fn remembering_by_default() -> bool {
    true
}

fn first_version() -> u32 {
    VERSION
}

/// Every connection's history and favourites. Within a list the newest comes first.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Wire", into = "Wire")]
pub struct QueryHistory {
    history: BTreeMap<NsKey, Vec<Entry>>,
    favourites: BTreeMap<Uuid, Vec<Favourite>>,
    remember: bool,
}

impl Default for QueryHistory {
    fn default() -> Self {
        Self {
            history: BTreeMap::new(),
            favourites: BTreeMap::new(),
            remember: true,
        }
    }
}

/// Only counts, for the same reason as [`FindText`].
impl std::fmt::Debug for QueryHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryHistory")
            .field("history", &self.history_len())
            .field(
                "favourites",
                &self.favourites.values().map(Vec::len).sum::<usize>(),
            )
            .finish()
    }
}

impl From<QueryHistory> for Wire {
    fn from(store: QueryHistory) -> Self {
        Self {
            version: VERSION,
            history: store
                .history
                .into_iter()
                .map(|((connection, db, collection), entries)| HistoryGroup {
                    connection,
                    db,
                    collection,
                    entries,
                })
                .collect(),
            favourites: store
                .favourites
                .into_iter()
                .map(|(connection, items)| FavouriteGroup { connection, items })
                .collect(),
            remember: store.remember,
        }
    }
}

impl TryFrom<Wire> for QueryHistory {
    type Error = String;

    /// A file that breaks a limit the API enforces is refused whole, never trimmed: dropping
    /// entries on load would lose favourites silently, while a refusal makes the app back the
    /// file up and say so. Names are trimmed; everything else must already be valid.
    fn try_from(wire: Wire) -> Result<Self, String> {
        if wire.version > VERSION {
            return Err(SavedError::Unsupported.error_variant().into());
        }
        let refuse = |e: SavedError| e.error_variant().to_string();
        let mut history = BTreeMap::new();
        for group in wire.history {
            checked_entries(&group.entries).map_err(refuse)?;
            if !group.entries.is_empty() {
                let key = (group.connection, group.db, group.collection);
                if history.insert(key, group.entries).is_some() {
                    return Err(refuse(SavedError::NameTaken));
                }
            }
        }
        let mut favourites = BTreeMap::new();
        for group in wire.favourites {
            let items = checked_favourites(group.items).map_err(refuse)?;
            if !items.is_empty() && favourites.insert(group.connection, items).is_some() {
                return Err(refuse(SavedError::NameTaken));
            }
        }
        Ok(Self {
            // A file that says off holds no history, whatever else it carries.
            history: if wire.remember {
                history
            } else {
                BTreeMap::new()
            },
            favourites,
            remember: wire.remember,
        })
    }
}

fn checked_entries(entries: &[Entry]) -> Result<(), SavedError> {
    if entries.len() > MAX_HISTORY_PER_NAMESPACE {
        return Err(SavedError::TooMany);
    }
    match entries.iter().any(|e| e.text.bytes() > MAX_ENTRY_BYTES) {
        true => Err(SavedError::TooLarge),
        false => Ok(()),
    }
}

fn checked_favourites(items: Vec<Favourite>) -> Result<Vec<Favourite>, SavedError> {
    if items.len() > MAX_FAVOURITES_PER_CONNECTION {
        return Err(SavedError::TooMany);
    }
    let items = items
        .into_iter()
        .map(|f| {
            let name = validate_name(&f.name)?;
            match f.text.bytes() > MAX_ENTRY_BYTES {
                true => Err(SavedError::TooLarge),
                false => Ok(Favourite { name, ..f }),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for f in &items {
        let name = (f.db.clone(), f.collection.clone(), f.name.to_lowercase());
        if !ids.insert(f.id) || !names.insert(name) {
            return Err(SavedError::NameTaken);
        }
    }
    Ok(items)
}

impl QueryHistory {
    /// The queries run in this namespace, most recent first.
    pub fn history(&self, connection: Uuid, db: &str, collection: &str) -> &[Entry] {
        self.history
            .get(&(connection, db.to_string(), collection.to_string()))
            .map_or(&[], Vec::as_slice)
    }

    /// How many history entries the store holds, over every connection and namespace.
    pub fn history_len(&self) -> usize {
        self.history.values().map(Vec::len).sum()
    }

    /// The connection's favourites (every namespace), most recently saved first.
    pub fn favourites(&self, connection: Uuid) -> &[Favourite] {
        self.favourites.get(&connection).map_or(&[], Vec::as_slice)
    }

    /// Records a query that was sent. A blank bar is not recorded (the store comes back as it
    /// was). A repeat of the same text and options moves to the front with the new time; the
    /// list keeps the newest [`MAX_HISTORY_PER_NAMESPACE`]. A query over [`MAX_ENTRY_BYTES`] is
    /// refused with `TooLarge`.
    pub fn recorded(
        &self,
        connection: Uuid,
        db: &str,
        collection: &str,
        text: FindText,
        now_unix: i64,
    ) -> Result<Self, SavedError> {
        if text.bytes() > MAX_ENTRY_BYTES {
            return Err(SavedError::TooLarge);
        }
        if text.is_blank() || !self.remember {
            return Ok(self.clone());
        }
        let entry = Entry {
            text,
            ran_unix: now_unix,
        };
        let list: Vec<Entry> = std::iter::once(entry.clone())
            .chain(
                self.history(connection, db, collection)
                    .iter()
                    .filter(|e| e.text != entry.text)
                    .cloned(),
            )
            .take(MAX_HISTORY_PER_NAMESPACE)
            .collect();
        Ok(self.with_history(connection, db, collection, list))
    }

    /// Saves a named copy of `text` for this namespace. The name is trimmed and must be unique
    /// among the namespace's favourites ignoring case.
    pub fn starred(
        &self,
        connection: Uuid,
        db: &str,
        collection: &str,
        name: &str,
        text: FindText,
        now_unix: i64,
    ) -> Result<Self, SavedError> {
        let name = validate_name(name)?;
        if text.bytes() > MAX_ENTRY_BYTES {
            return Err(SavedError::TooLarge);
        }
        if self.name_taken(connection, db, collection, &name, None) {
            return Err(SavedError::NameTaken);
        }
        if self.favourites(connection).len() >= MAX_FAVOURITES_PER_CONNECTION {
            return Err(SavedError::TooMany);
        }
        let favourite = Favourite {
            id: Uuid::new_v4(),
            name,
            db: db.to_string(),
            collection: collection.to_string(),
            text,
            saved_unix: now_unix,
        };
        let list = std::iter::once(favourite)
            .chain(self.favourites(connection).iter().cloned())
            .collect();
        Ok(self.with_favourites(connection, list))
    }

    /// Gives a favourite a new name, keeping its place and text. An unknown id changes nothing.
    pub fn renamed(&self, connection: Uuid, id: Uuid, name: &str) -> Result<Self, SavedError> {
        let name = validate_name(name)?;
        let Some(target) = self.favourites(connection).iter().find(|f| f.id == id) else {
            return Ok(self.clone());
        };
        if self.name_taken(connection, &target.db, &target.collection, &name, Some(id)) {
            return Err(SavedError::NameTaken);
        }
        let list = self
            .favourites(connection)
            .iter()
            .map(|f| match f.id == id {
                true => Favourite {
                    name: name.clone(),
                    ..f.clone()
                },
                false => f.clone(),
            })
            .collect();
        Ok(self.with_favourites(connection, list))
    }

    /// Removes one favourite. An unknown id changes nothing.
    pub fn unstarred(&self, connection: Uuid, id: Uuid) -> Self {
        let list = self
            .favourites(connection)
            .iter()
            .filter(|f| f.id != id)
            .cloned()
            .collect();
        self.with_favourites(connection, list)
    }

    /// Forgets the history of one namespace; favourites stay.
    pub fn cleared(&self, connection: Uuid, db: &str, collection: &str) -> Self {
        self.with_history(connection, db, collection, Vec::new())
    }

    /// Forgets every history entry of every connection; favourites stay.
    pub fn history_erased(&self) -> Self {
        Self {
            history: BTreeMap::new(),
            ..self.clone()
        }
    }

    /// Whether finds that are run are recorded (the "Remember queries" switch).
    pub fn remembering(&self) -> bool {
        self.remember
    }

    /// The switch set. Off erases the history at once (it is the person's typed text);
    /// favourites stay because they were made on purpose. On starts from an empty history.
    pub fn with_remembering(&self, on: bool) -> Self {
        Self {
            remember: on,
            ..self.history_erased()
        }
    }

    /// For a deleted connection: its history and its favourites go.
    pub fn without_connection(&self, connection: Uuid) -> Self {
        Self {
            history: self
                .history
                .iter()
                .filter(|(key, _)| key.0 != connection)
                .map(|(key, list)| (key.clone(), list.clone()))
                .collect(),
            favourites: self
                .favourites
                .iter()
                .filter(|(id, _)| **id != connection)
                .map(|(id, list)| (*id, list.clone()))
                .collect(),
            remember: self.remember,
        }
    }

    /// For a duplicated connection: `to` starts with the favourites of `from`, never its
    /// history. Unchanged when `from` has none.
    pub fn with_connection_copied(&self, from: Uuid, to: Uuid) -> Self {
        match self.favourites(from) {
            [] => self.clone(),
            list => self.with_favourites(to, list.to_vec()),
        }
    }

    /// The store without its single oldest history entry (any namespace), to make room in a
    /// file; `None` when there is no history left. Favourites are never dropped.
    pub fn oldest_history_dropped(&self) -> Option<Self> {
        let (key, _) = self
            .history
            .iter()
            .filter_map(|(key, list)| list.last().map(|e| (key, e.ran_unix)))
            .min_by_key(|(_, ran)| *ran)?;
        let mut list = self.history.get(key)?.clone();
        list.pop();
        Some(self.with_history(key.0, &key.1, &key.2, list))
    }

    fn name_taken(
        &self,
        connection: Uuid,
        db: &str,
        collection: &str,
        name: &str,
        except: Option<Uuid>,
    ) -> bool {
        self.favourites(connection).iter().any(|f| {
            Some(f.id) != except
                && f.db == db
                && f.collection == collection
                && same_name(&f.name, name)
        })
    }

    /// The store with one namespace's list replaced; an empty list drops the entry.
    fn with_history(&self, connection: Uuid, db: &str, collection: &str, list: Vec<Entry>) -> Self {
        let mut history = self.history.clone();
        let key = (connection, db.to_string(), collection.to_string());
        match list.is_empty() {
            true => history.remove(&key),
            false => history.insert(key, list),
        };
        Self {
            history,
            ..self.clone()
        }
    }

    /// The store with one connection's favourites replaced; an empty list drops the entry.
    fn with_favourites(&self, connection: Uuid, list: Vec<Favourite>) -> Self {
        let mut favourites = self.favourites.clone();
        match list.is_empty() {
            true => favourites.remove(&connection),
            false => favourites.insert(connection, list),
        };
        Self {
            favourites,
            ..self.clone()
        }
    }
}

#[cfg(test)]
#[path = "query_history_tests.rs"]
mod tests;
