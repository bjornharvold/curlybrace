//! Which executable to use for each tool: a global default and per-connection overrides.
//!
//! Overrides are keyed by connection id and live here, never on `ConnectionConfig`, so an
//! exported connection file carries no executable paths. Every change returns a new value.
use super::{ToolChoice, ToolKind};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

type Choices = BTreeMap<ToolKind, ToolChoice>;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSettings {
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "lenient_choices"
    )]
    global: Choices,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "lenient_connections"
    )]
    connections: BTreeMap<Uuid, Choices>,
}

/// Reads the choices of the tools this build knows. A tool it does not know (a newer build also
/// manages `mongoexport`, say) is skipped, so going back to an older build keeps every known
/// entry instead of treating the whole file as unusable; the unknown one is gone at the next
/// save. A known tool with a choice that makes no sense is still an error.
fn lenient_choices<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Choices, D::Error> {
    let raw = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
    let mut choices = Choices::new();
    for (name, value) in raw {
        let Ok(kind) = serde_json::from_value::<ToolKind>(serde_json::Value::String(name)) else {
            log::warn!("the tool paths name a tool this version does not know; ignoring it");
            continue;
        };
        let choice =
            serde_json::from_value::<ToolChoice>(value).map_err(serde::de::Error::custom)?;
        choices.insert(kind, choice);
    }
    Ok(choices)
}

/// A connection's choices through [`lenient_choices`].
struct Lenient(Choices);

impl<'de> Deserialize<'de> for Lenient {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        lenient_choices(deserializer).map(Self)
    }
}

fn lenient_connections<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<Uuid, Choices>, D::Error> {
    let raw = BTreeMap::<Uuid, Lenient>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .map(|(id, Lenient(choices))| (id, choices))
        .filter(|(_, choices)| !choices.is_empty())
        .collect())
}

impl ToolSettings {
    /// The connection's own choice, else the global one, else the `PATH` search.
    pub fn effective(&self, kind: ToolKind, connection: Option<Uuid>) -> ToolChoice {
        connection
            .and_then(|id| self.connections.get(&id))
            .and_then(|choices| choices.get(&kind))
            .or_else(|| self.global.get(&kind))
            .cloned()
            .unwrap_or(ToolChoice::Path)
    }

    /// The global default alone (what a connection without an override uses).
    pub fn global(&self, kind: ToolKind) -> ToolChoice {
        self.global.get(&kind).cloned().unwrap_or(ToolChoice::Path)
    }

    /// Whether the connection has its own choice for `kind` ("use global settings" is off).
    pub fn overrides(&self, connection: Uuid, kind: ToolKind) -> bool {
        self.connections
            .get(&connection)
            .is_some_and(|choices| choices.contains_key(&kind))
    }

    pub fn with_global(&self, kind: ToolKind, choice: ToolChoice) -> Self {
        let mut global = self.global.clone();
        global.insert(kind, choice);
        Self {
            global,
            ..self.clone()
        }
    }

    pub fn with_override(&self, connection: Uuid, kind: ToolKind, choice: ToolChoice) -> Self {
        let mut connections = self.connections.clone();
        connections
            .entry(connection)
            .or_default()
            .insert(kind, choice);
        Self {
            connections,
            ..self.clone()
        }
    }

    /// Back to the global choice for this tool; an emptied connection entry is dropped.
    pub fn without_override(&self, connection: Uuid, kind: ToolKind) -> Self {
        let mut connections = self.connections.clone();
        if let Some(choices) = connections.get_mut(&connection) {
            choices.remove(&kind);
            if choices.is_empty() {
                connections.remove(&connection);
            }
        }
        Self {
            connections,
            ..self.clone()
        }
    }

    /// For a deleted connection.
    pub fn without_connection(&self, connection: Uuid) -> Self {
        let mut connections = self.connections.clone();
        connections.remove(&connection);
        Self {
            connections,
            ..self.clone()
        }
    }

    /// For a duplicated connection: `to` starts with the overrides of `from`. Unchanged when
    /// `from` has none.
    pub fn with_connection_copied(&self, from: Uuid, to: Uuid) -> Self {
        let Some(choices) = self.connections.get(&from) else {
            return self.clone();
        };
        let mut connections = self.connections.clone();
        connections.insert(to, choices.clone());
        Self {
            connections,
            ..self.clone()
        }
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
