//! Sidebar view: database tree with lazily loaded collections.
use crate::I18n;
use crate::viewmodel::{
    DbNode, PasteTarget, SidebarState, collection_click_action, compare_database_to_action,
    compare_to_action, db_click_action,
};
use crate::views::sidebar_copy::{
    TreeCtx, collection_node, copy_paste_entries, database_node, is_selected, node_action,
    row_paste_target,
};
use crate::views::sidebar_drag::{drop_target, row_button, start_drag};
use curlybrace_core::store::Namespace;
use eframe::egui;
use std::ops::Range;

pub use crate::viewmodel::SidebarAction;

/// One visible line of the flattened tree; borrows from the state (no copies).
#[derive(Debug, PartialEq)]
enum Row<'a> {
    Db(&'a DbNode),
    Collection {
        db: &'a str,
        name: &'a str,
    },
    /// Expanded database whose collections are still loading.
    Loading,
    /// Expanded database with no collections.
    NoCollections,
}

/// Height of every rendered row; the single source of truth for `show_rows` and the rows.
fn row_height(ui: &egui::Ui) -> f32 {
    ui.spacing().interact_size.y
}

/// Rows a database occupies: its header plus children or one placeholder when expanded.
fn rows_of(node: &DbNode) -> usize {
    1 + match (&node.collections, node.expanded) {
        (_, false) => 0,
        (Some(c), true) => c.len().max(1),
        (None, true) => 1,
    }
}

fn total_rows(state: &SidebarState) -> usize {
    state.databases.iter().map(rows_of).sum()
}

/// The rows in `range` of the flattened tree, skipping whole collapsed subtrees.
fn visible_rows(state: &SidebarState, range: Range<usize>) -> Vec<Row<'_>> {
    let mut out = Vec::with_capacity(range.len().min(total_rows(state)));
    let mut start = 0;
    for node in &state.databases {
        if start >= range.end {
            break;
        }
        let end = start + rows_of(node);
        if end > range.start {
            push_node_rows(node, start, &range, &mut out);
        }
        start = end;
    }
    out
}

fn push_node_rows<'a>(
    node: &'a DbNode,
    first: usize,
    range: &Range<usize>,
    out: &mut Vec<Row<'a>>,
) {
    let mut push = |offset: usize, row: Row<'a>| {
        if range.contains(&(first + offset)) {
            out.push(row);
        }
    };
    push(0, Row::Db(node));
    if !node.expanded {
        return;
    }
    match node.collections.as_deref() {
        None => push(1, Row::Loading),
        Some([]) => push(1, Row::NoCollections),
        Some(names) => {
            for (i, name) in names.iter().enumerate() {
                let row = Row::Collection {
                    db: &node.name,
                    name,
                };
                push(1 + i, row);
            }
        }
    }
}

/// The text of a database row: an expand/collapse arrow and the name (one place, shared with the
/// UI tests so they do not hardcode the glyphs).
pub fn db_row_label(name: &str, expanded: bool) -> String {
    let arrow = if expanded { "▾" } else { "▸" };
    format!("{arrow} {name}")
}

/// Height that fits the heading line and up to `max_rows` rows of `state`'s tree, so a root's
/// tree can sit in a bounded area (the rows themselves stay virtualised).
pub fn preferred_height(ui: &egui::Ui, state: &SidebarState, max_rows: usize) -> f32 {
    let heading = ui
        .text_style_height(&egui::TextStyle::Heading)
        .max(ui.spacing().interact_size.y);
    tree_height(ui, state, max_rows, heading)
}

/// Like [`preferred_height`] for [`show_sidebar_body`]: a compact Refresh row instead of the
/// heading line.
pub fn preferred_body_height(ui: &egui::Ui, state: &SidebarState, max_rows: usize) -> f32 {
    tree_height(ui, state, max_rows, ui.spacing().interact_size.y)
}

fn tree_height(ui: &egui::Ui, state: &SidebarState, max_rows: usize, top_row: f32) -> f32 {
    let spacing = ui.spacing();
    let rows = total_rows(state).clamp(1, max_rows) as f32;
    top_row + spacing.item_spacing.y * 2.0 + rows * (row_height(ui) + spacing.item_spacing.y)
}

/// The sidebar of a single session: the large "Databases" heading with Refresh, then the tree.
pub fn show_sidebar(ui: &mut egui::Ui, state: &SidebarState, i18n: &I18n) -> Option<SidebarAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.heading(i18n.t("sidebar.title"));
        if ui.button(i18n.t("sidebar.refresh")).clicked() {
            action = Some(SidebarAction::Refresh);
        }
    });
    action.or(show_tree_rows(ui, state, None, i18n))
}

/// A root's tree without the heading: a compact, right-aligned Refresh, then the tree.
pub fn show_sidebar_body(
    ui: &mut egui::Ui,
    state: &SidebarState,
    ctx: Option<&TreeCtx>,
    i18n: &I18n,
) -> Option<SidebarAction> {
    // A right-aligned layout would claim the whole remaining height: give the row its own.
    let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
    let refresh = ui
        .allocate_ui_with_layout(
            row,
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| ui.small_button(i18n.t("sidebar.refresh")).clicked(),
        )
        .inner;
    let action = show_tree_rows(ui, state, ctx, i18n);
    refresh.then_some(SidebarAction::Refresh).or(action)
}

fn show_tree_rows(
    ui: &mut egui::Ui,
    state: &SidebarState,
    ctx: Option<&TreeCtx>,
    i18n: &I18n,
) -> Option<SidebarAction> {
    let mut action = None;
    if state.databases.is_empty() {
        ui.label(i18n.t("sidebar.empty"));
        return action;
    }
    let row_height = row_height(ui);
    egui::ScrollArea::vertical()
        .id_salt("sidebar_tree")
        .auto_shrink([false; 2])
        .show_rows(ui, row_height, total_rows(state), |ui, range| {
            for row in visible_rows(state, range) {
                if let Some(a) = show_fixed_row(ui, state, row, ctx, row_height, i18n) {
                    action = Some(a);
                }
            }
        });
    action
}

/// Renders one row occupying exactly `row_height`, so virtualised scrolling stays exact.
fn show_fixed_row(
    ui: &mut egui::Ui,
    state: &SidebarState,
    row: Row<'_>,
    ctx: Option<&TreeCtx>,
    row_height: f32,
    i18n: &I18n,
) -> Option<SidebarAction> {
    let size = egui::vec2(ui.available_width(), row_height);
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(size, layout, |ui| {
        ui.set_min_height(row_height);
        show_row(ui, state, row, ctx, i18n)
    })
    .inner
}

fn show_row(
    ui: &mut egui::Ui,
    state: &SidebarState,
    row: Row<'_>,
    ctx: Option<&TreeCtx>,
    i18n: &I18n,
) -> Option<SidebarAction> {
    match row {
        Row::Db(node) => {
            let node_ref = database_node(ctx, &node.name);
            let selected = is_selected(ctx, &node_ref);
            let label = db_row_label(&node.name, node.expanded);
            let response = row_button(ui, ctx, selected, label);
            start_drag(&response, ctx, &node_ref);
            let target = database_target(ctx, &node.name);
            let dropped = target.and_then(|t| drop_target(&response, ctx, t, i18n));
            let menu = database_menu(&response, &node.name, ctx, i18n);
            dropped
                .or(menu)
                .or_else(|| node_action(ui, &response, ctx, &node_ref, || db_click_action(node)))
        }
        Row::Collection { db, name } => {
            let node_ref = collection_node(ctx, db, name);
            let opened = state
                .selected
                .as_ref()
                .is_some_and(|ns| ns.db == db && ns.collection == name);
            ui.add_space(ui.spacing().indent);
            let response = row_button(ui, ctx, opened || is_selected(ctx, &node_ref), name);
            start_drag(&response, ctx, &node_ref);
            let target = collection_target(ctx, db, name);
            let dropped = target.and_then(|t| drop_target(&response, ctx, t, i18n));
            let menu = collection_menu(&response, db, name, ctx, i18n);
            dropped.or(menu).or_else(|| {
                node_action(ui, &response, ctx, &node_ref, || {
                    collection_click_action(db, name)
                })
            })
        }
        Row::Loading => placeholder(ui, &i18n.t("sidebar.loading")),
        Row::NoCollections => placeholder(ui, &i18n.t("sidebar.no_collections")),
    }
}

/// Where a paste or drop on the database row `db` lands (none without a selection model).
fn database_target(ctx: Option<&TreeCtx>, db: &str) -> Option<PasteTarget> {
    ctx.map(|c| PasteTarget::Database {
        session: c.session,
        db: db.to_string(),
    })
}

/// Like [`database_target`] for a collection row.
fn collection_target(ctx: Option<&TreeCtx>, db: &str, name: &str) -> Option<PasteTarget> {
    ctx.map(|c| PasteTarget::Collection {
        session: c.session,
        ns: Namespace::new(db, name),
    })
}

/// Right-click menu of a database: compare, copy the selection, paste onto it.
fn database_menu(
    response: &egui::Response,
    db: &str,
    ctx: Option<&TreeCtx>,
    i18n: &I18n,
) -> Option<SidebarAction> {
    let mut chosen = None;
    response.context_menu(|ui| {
        if ui.button(i18n.t("sidebar.compare_database_to")).clicked() {
            chosen = Some(compare_database_to_action(db));
            ui.close();
        }
        let target = database_target(ctx, db);
        let copy_paste = target.and_then(|t| copy_paste_entries(ui, ctx, t, i18n));
        chosen = copy_paste.or(chosen.take());
    });
    chosen
}

/// Right-click menu of a collection: compare, copy the selection, paste onto it (which only
/// documents allow; the paste rules say why a clipboard of collections cannot).
fn collection_menu(
    response: &egui::Response,
    db: &str,
    name: &str,
    ctx: Option<&TreeCtx>,
    i18n: &I18n,
) -> Option<SidebarAction> {
    let mut chosen = None;
    response.context_menu(|ui| {
        if ui.button(i18n.t("sidebar.compare_to")).clicked() {
            chosen = Some(compare_to_action(db, name));
            ui.close();
        }
        let target = ctx.map(|c| row_paste_target(c, &collection_node(ctx, db, name)));
        let copy_paste = target.and_then(|t| copy_paste_entries(ui, ctx, t, i18n));
        chosen = copy_paste.or(chosen.take());
    });
    chosen
}

/// Non-clickable indented hint row.
fn placeholder(ui: &mut egui::Ui, text: &str) -> Option<SidebarAction> {
    ui.add_space(ui.spacing().indent);
    ui.weak(text);
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use curlybrace_core::store::Namespace;

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
    fn flattening_counts_children_and_placeholders() {
        let s = populated();
        // "my db": 1 + 3, "a.b.c" collapsed: 1, "данные" collapsed: 1
        assert_eq!(total_rows(&s), 4 + 1 + 1);
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
    fn expanded_loading_and_empty_dbs_get_one_placeholder_row() {
        let s = SidebarState::default()
            .with_databases(names(&["loading", "empty", "closed"]))
            .toggled("loading")
            .toggled("empty")
            .with_collections("empty", vec![])
            .with_collections("closed", vec![]);
        assert_eq!(total_rows(&s), 2 + 2 + 1);
        let all = visible_rows(&s, 0..5);
        assert!(matches!(all[1], Row::Loading));
        assert!(matches!(all[3], Row::NoCollections));
        assert!(matches!(all[4], Row::Db(n) if n.name == "closed"));
        assert_eq!(all.len(), total_rows(&s));
        assert_eq!(run_frame(&s), None);
    }

    #[test]
    fn visible_window_is_a_slice_of_the_flattened_tree() {
        let s = populated();
        let all = visible_rows(&s, 0..6);
        assert_eq!(visible_rows(&s, 2..5), all[2..5]);
        assert!(visible_rows(&s, 6..20).is_empty());
    }

    fn big_state() -> SidebarState {
        let dbs: Vec<String> = (0..2000).map(|i| format!("db {i}.x")).collect();
        let mut s = SidebarState::default().with_databases(dbs);
        for i in [3, 700, 1999] {
            let db = format!("db {i}.x");
            let colls = (0..500).map(|c| format!("c{c}")).collect();
            s = s.toggled(&db).with_collections(&db, colls);
        }
        s
    }

    #[test]
    fn row_math_matches_for_any_window_of_a_large_tree() {
        let s = big_state();
        let total = total_rows(&s);
        assert_eq!(total, 2000 + 3 * 500);
        for start in [0, 2, 3, 500, 1200, total - 10] {
            let window = start..start + 40;
            let rows = visible_rows(&s, window.clone());
            assert_eq!(rows.len(), window.end.min(total) - start);
            let h = 18.0_f32;
            assert_eq!(
                rows.len() as f32 * h,
                (window.end.min(total) - start) as f32 * h
            );
        }
        let sum: usize = s.databases.iter().map(rows_of).sum();
        assert_eq!(sum as f32 * 18.0, total as f32 * 18.0);
    }

    #[test]
    fn large_tree_renders_in_a_fixed_viewport_with_a_small_window() {
        let s = big_state();
        let ctx = egui::Context::default();
        let i18n = I18n::english();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(300.0, 400.0),
            )),
            ..Default::default()
        };
        let mut out = None;
        let mut row_h = 0.0;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                row_h = row_height(ui) + ui.spacing().item_spacing.y;
                out = show_sidebar(ui, &s, &i18n);
            });
        });
        assert_eq!(out, None);
        let window = (400.0 / row_h).ceil() as usize + 1;
        assert!(visible_rows(&s, 0..window).len() < 100);
        assert!(window < total_rows(&s) / 20);
    }
}
