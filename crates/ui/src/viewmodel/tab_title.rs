//! The words on a tab: its title, tooltip and accessible name (#159).
//!
//! A title is the collection name. When two shown tabs have the same collection name in different
//! databases it is `db.collection`; when two shown tabs still share a title but not a connection
//! it ends with the connection. Tabs on the very same collection of the same connection
//! (duplicates) share a title; their accessible names tell them apart by position.
use super::ConnectionBadge;
use crate::I18n;
use crate::app::state::AppState;
use crate::app::tab_queries::TabGlance;
use crate::app::tabs::{Freshness, TabId};
use std::collections::BTreeSet;
use uuid::Uuid;

/// What the strip shows and says for one tab.
#[derive(Clone, Debug, PartialEq)]
pub struct TabLabel {
    pub title: String,
    pub tooltip: String,
    pub accessible: String,
    /// The name of the tab's entry in the All tabs menu: the title and its place, so duplicates
    /// differ.
    pub menu_name: String,
    pub badge: ConnectionBadge,
    /// Its connection was lost and it has not run since: it shows the Run row.
    pub dormant: bool,
    /// An editor or a modal of the index writes is open in it.
    pub unsaved: bool,
}

struct Row {
    tab: TabId,
    session: Uuid,
    glance: TabGlance,
}

/// The labels of the tabs the strip shows, in strip order.
pub fn labels(state: &AppState, i18n: &I18n) -> Vec<(TabId, TabLabel)> {
    let rows: Vec<Row> = state
        .visible_tabs()
        .into_iter()
        .filter_map(|tab| {
            let session = state.tabs.owner.get(&tab).copied()?;
            Some(Row {
                tab,
                session,
                glance: state.glance(tab)?,
            })
        })
        .collect();
    let bases: Vec<String> = rows.iter().map(|r| base_title(&rows, r, i18n)).collect();
    let count = rows.len().to_string();
    rows.iter()
        .enumerate()
        .map(|(at, row)| {
            let connection = state.open_label(row.session, i18n).unwrap_or_default();
            let title = final_title(&rows, &bases, at, &connection, i18n);
            let label = label_of(state, row, (title, &connection), (at + 1, &count), i18n);
            (row.tab, label)
        })
        .collect()
}

/// The collection name, or `db.collection` when another shown tab has that name in another
/// database; the empty tab's title.
fn base_title(rows: &[Row], row: &Row, i18n: &I18n) -> String {
    let Some(ns) = row.glance.ns.as_ref() else {
        return i18n.t("tabs.empty_title");
    };
    let clash = rows
        .iter()
        .filter_map(|r| r.glance.ns.as_ref())
        .any(|other| other.collection == ns.collection && other.db != ns.db);
    match clash {
        true => i18n.t_with(
            "tabs.title_with_db",
            &[("db", &ns.db), ("collection", &ns.collection)],
        ),
        false => ns.collection.clone(),
    }
}

/// `bases[at]`, with the connection when another shown tab has the same title on another
/// connection.
fn final_title(rows: &[Row], bases: &[String], at: usize, connection: &str, i18n: &I18n) -> String {
    let base = &bases[at];
    if rows[at].glance.ns.is_none() {
        return base.clone();
    }
    let sessions: BTreeSet<Uuid> = rows
        .iter()
        .zip(bases)
        .filter(|(_, b)| *b == base)
        .map(|(r, _)| r.session)
        .collect();
    match sessions.len() > 1 {
        true => i18n.t_with(
            "tabs.title_with_connection",
            &[("collection", base), ("connection", connection)],
        ),
        false => base.clone(),
    }
}

fn label_of(
    state: &AppState,
    row: &Row,
    (title, connection): (String, &str),
    (position, count): (usize, &str),
    i18n: &I18n,
) -> TabLabel {
    let badge = state.session_badge(row.session);
    let dormant = state.freshness_of(row.tab) == Freshness::Dormant;
    let namespace = row.glance.ns.as_ref().map_or_else(
        || title.clone(),
        |ns| {
            i18n.t_with(
                "tabs.title_with_db",
                &[("db", &ns.db), ("collection", &ns.collection)],
            )
        },
    );
    let sub = i18n.t(row.glance.sub.label_key());
    // What the tab is in, as whole phrases of the language joined by its list separator.
    let states = [
        badge.production.then(|| i18n.t("tabs.state.production")),
        badge.read_only.then(|| i18n.t("tabs.state.read_only")),
        row.glance.unsaved.then(|| i18n.t("tabs.state.unsaved")),
        dormant.then(|| i18n.t("tabs.state.needs_run")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(&i18n.t("connections.list_separator"));
    let details = [
        ("connection", connection),
        ("namespace", namespace.as_str()),
        ("sub", sub.as_str()),
        ("state", states.as_str()),
    ];
    let tooltip = match states.is_empty() {
        true => i18n.t_with("tabs.tooltip", &details),
        false => i18n.t_with("tabs.tooltip_with_state", &details),
    };
    let position = position.to_string();
    let place = [("position", position.as_str()), ("count", count)];
    let named = [&[("title", title.as_str())][..], &details, &place].concat();
    let accessible = match states.is_empty() {
        true => i18n.t_with("tabs.accessible_name", &named),
        false => i18n.t_with("tabs.accessible_name_with_state", &named),
    };
    let menu_name = i18n.t_with(
        "tabs.menu_entry",
        &[("title", &title), ("position", &position), ("count", count)],
    );
    TabLabel {
        title,
        tooltip,
        accessible,
        menu_name,
        badge,
        dormant,
        unsaved: row.glance.unsaved,
    }
}

#[cfg(test)]
#[path = "tab_title_tests.rs"]
mod tests;
