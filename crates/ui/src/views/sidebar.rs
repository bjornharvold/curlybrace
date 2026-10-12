//! Sidebar view: database tree with lazily loaded collections.
use crate::I18n;
use crate::viewmodel::{DbNode, SidebarState};
use eframe::egui;
use mongostudio_core::store::Namespace;
use std::ops::Range;

/// What the user asked for; the caller performs it.
#[derive(Debug, Clone, PartialEq)]
pub enum SidebarAction {
    /// The user expanded a database whose collections are not loaded: the caller should
    /// toggle it open (`SidebarState::toggled`) and fetch its collections.
    LoadCollections(String),
    /// The user collapsed or re-expanded an already loaded database: the caller toggles it.
    Toggle(String),
    Open(Namespace),
    Refresh,
}

/// One visible line of the flattened tree; borrows from the state (no copies).
#[derive(Debug, PartialEq)]
enum Row<'a> {
    Db(&'a DbNode),
    Collection { db: &'a str, name: &'a str },
}

fn rows_of(node: &DbNode) -> usize {
    1 + match (&node.collections, node.expanded) {
        (Some(c), true) => c.len(),
        _ => 0,
    }
}

fn total_rows(state: &SidebarState) -> usize {
    state.databases.iter().map(rows_of).sum()
}

/// The rows in `range` of the flattened tree, skipping whole collapsed subtrees.
fn visible_rows(state: &SidebarState, range: Range<usize>) -> Vec<Row<'_>> {
    let mut out = Vec::with_capacity(range.len());
    let mut start = 0;
    for node in &state.databases {
        let end = start + rows_of(node);
        if end > range.start && start < range.end {
            push_node_rows(node, start, &range, &mut out);
        }
        start = end;
        if start >= range.end {
            break;
        }
    }
    out
}

fn push_node_rows<'a>(
    node: &'a DbNode,
    first: usize,
    range: &Range<usize>,
    out: &mut Vec<Row<'a>>,
) {
    if range.contains(&first) {
        out.push(Row::Db(node));
    }
    let children = node.collections.iter().flatten().take(rows_of(node) - 1);
    for (i, name) in children.enumerate() {
        if range.contains(&(first + 1 + i)) {
            out.push(Row::Collection {
                db: &node.name,
                name,
            });
        }
    }
}

pub fn show_sidebar(ui: &mut egui::Ui, state: &SidebarState, i18n: &I18n) -> Option<SidebarAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.heading(i18n.t("sidebar.title"));
        if ui.button(i18n.t("sidebar.refresh")).clicked() {
            action = Some(SidebarAction::Refresh);
        }
    });
    if state.databases.is_empty() {
        ui.label(i18n.t("sidebar.empty"));
        return action;
    }
    let row_height = ui.text_style_height(&egui::TextStyle::Button);
    egui::ScrollArea::vertical()
        .id_salt("sidebar_tree")
        .auto_shrink([false; 2])
        .show_rows(ui, row_height, total_rows(state), |ui, range| {
            for row in visible_rows(state, range) {
                if let Some(a) = show_row(ui, state, row) {
                    action = Some(a);
                }
            }
        });
    action
}

fn show_row(ui: &mut egui::Ui, state: &SidebarState, row: Row<'_>) -> Option<SidebarAction> {
    match row {
        Row::Db(node) => {
            let arrow = if node.expanded { "▾" } else { "▸" };
            let label = format!("{arrow} {}", node.name);
            if !ui.selectable_label(false, label).clicked() {
                return None;
            }
            let load = !node.expanded && node.collections.is_none();
            Some(if load {
                SidebarAction::LoadCollections(node.name.clone())
            } else {
                SidebarAction::Toggle(node.name.clone())
            })
        }
        Row::Collection { db, name } => {
            let selected = state
                .selected
                .as_ref()
                .is_some_and(|ns| ns.db == db && ns.collection == name);
            ui.horizontal(|ui| {
                ui.add_space(ui.spacing().indent);
                ui.selectable_label(selected, name).clicked()
            })
            .inner
            .then(|| SidebarAction::Open(Namespace::new(db, name)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn populated() -> SidebarState {
        SidebarState::default()
            .with_databases(names(&["my db", "a.b.c", "данные"]))
            .toggled("my db")
            .with_collections("my db", names(&["коллекция", "x.y", "with space"]))
            .with_collections("a.b.c", names(&["hidden"]))
            .selecting(Namespace::new("my db", "коллекция"))
    }

    fn run_frame(state: &SidebarState) -> Option<SidebarAction> {
        let ctx = egui::Context::default();
        let i18n = I18n::english();
        let mut out = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                out = show_sidebar(ui, state, &i18n);
            });
        });
        out
    }

    #[test]
    fn renders_empty_and_populated_state_without_panicking() {
        assert_eq!(run_frame(&SidebarState::default()), None);
        assert_eq!(run_frame(&populated()), None);
    }

    #[test]
    fn flattening_counts_only_expanded_loaded_collections() {
        let s = populated();
        assert_eq!(total_rows(&s), 3 + 3);
        let all = visible_rows(&s, 0..total_rows(&s));
        assert_eq!(all.len(), 6);
        assert!(matches!(all[0], Row::Db(n) if n.name == "my db"));
        assert!(matches!(
            all[1],
            Row::Collection {
                db: "my db",
                name: "коллекция"
            }
        ));
        assert!(matches!(all[4], Row::Db(n) if n.name == "a.b.c"));
    }

    #[test]
    fn visible_window_is_a_slice_of_the_flattened_tree() {
        let s = populated();
        let all = visible_rows(&s, 0..6);
        assert_eq!(visible_rows(&s, 2..5), all[2..5]);
        assert!(visible_rows(&s, 6..20).is_empty());
    }
}
