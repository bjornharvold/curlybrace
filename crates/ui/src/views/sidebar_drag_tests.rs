use super::*;
use crate::app::reducer::Input;
use crate::app::session::SessionView;
use crate::app::state::{AppState, ConnStatus};
use crate::viewmodel::{PasteBlock, SidebarSelection, SidebarState, paste_block_key};
use crate::views::sidebar::db_row_label;
use crate::views::sidebar_roots::show_roots;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::store::Namespace;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use uuid::Uuid;

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

struct Fixture {
    state: AppState,
    a: Uuid,
    b: Uuid,
}

/// `Alpha` (active): `shop` with orders and users. `Beta` (parked): `crm` with contacts.
fn fixture() -> Fixture {
    let configs: Vec<_> = ["Alpha", "Beta"]
        .iter()
        .map(|n| ConnectionConfig::new(n, "mongodb://localhost"))
        .collect();
    let ids: Vec<Uuid> = configs.iter().map(|c| c.id).collect();
    let mut state = AppState::new(configs);
    state.open = ids.clone();
    state.active = Some(ids[0]);
    state.status = ConnStatus::Connected;
    state.sidebar = SidebarState::default()
        .with_databases(names(&["shop"]))
        .toggled("shop")
        .with_collections("shop", names(&["orders", "users"]));
    let parked = SidebarState::default()
        .with_databases(names(&["crm"]))
        .toggled("crm")
        .with_collections("crm", names(&["contacts"]));
    state.parked.insert(
        ids[1],
        SessionView {
            status: ConnStatus::Connected,
            sidebar: parked,
            ..SessionView::default()
        },
    );
    Fixture {
        state,
        a: ids[0],
        b: ids[1],
    }
}

fn harness(state: AppState) -> Harness<'static, (AppState, Option<Input>)> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(320.0, 900.0))
        .build_ui_state(
            |ui, (state, out): &mut (AppState, Option<Input>)| {
                *out = show_roots(ui, state, &I18n::english()).or(out.take());
            },
            (state, None),
        );
    harness.run();
    harness
}

fn collection(session: Uuid, name: &str) -> NodeRef {
    NodeRef::Collection {
        session,
        ns: Namespace::new("shop", name),
    }
}

fn pointer(harness: &mut Harness<'static, (AppState, Option<Input>)>, event: egui::Event) {
    harness.input_mut().events.push(event);
    harness.step();
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// Presses on the row labelled `from`, moves over the row labelled `to` and stays there.
fn drag_over(
    harness: &mut Harness<'static, (AppState, Option<Input>)>,
    from: &str,
    to: &str,
) -> egui::Pos2 {
    let start = harness.get_by_label(from).rect().center();
    let end = harness.get_by_label(to).rect().center();
    pointer(harness, egui::Event::PointerMoved(start));
    pointer(harness, button(start, true));
    pointer(
        harness,
        egui::Event::PointerMoved(start + egui::vec2(30.0, 0.0)),
    );
    pointer(harness, egui::Event::PointerMoved(end));
    pointer(harness, egui::Event::PointerMoved(end));
    end
}

fn release(harness: &mut Harness<'static, (AppState, Option<Input>)>, at: egui::Pos2) {
    pointer(harness, button(at, false));
    harness.run();
}

fn dropped(input: Option<Input>) -> (Option<Uuid>, SidebarAction) {
    match input {
        Some(Input::Sidebar(SidebarAction::InSession { session, action })) => {
            (Some(session), *action)
        }
        Some(Input::Sidebar(action)) => (None, action),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn dragging_an_unselected_row_onto_a_database_drops_that_row_alone() {
    let f = fixture();
    let (a, b) = (f.a, f.b);
    let mut state = f.state;
    // The selection is somewhere else: dragging `orders` takes only `orders`.
    state.selection = SidebarSelection::default().single(collection(a, "users"));
    let mut harness = harness(state);
    harness.state_mut().1 = None;
    let crm = db_row_label("crm", true);
    let end = drag_over(&mut harness, "orders", &crm);
    release(&mut harness, end);
    let (_, action) = dropped(harness.state_mut().1.take());
    match action {
        SidebarAction::Drop { clipboard, target } => {
            let expected = CopyClipboard::collections(a, "shop", names(&["orders"])).unwrap();
            assert_eq!(*clipboard, expected);
            assert_eq!(
                target,
                PasteTarget::Database {
                    session: b,
                    db: "crm".into()
                }
            );
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn dragging_a_selected_row_drops_the_whole_selection() {
    let f = fixture();
    let (a, b) = (f.a, f.b);
    let mut state = f.state;
    state.selection = SidebarSelection::default()
        .single(collection(a, "orders"))
        .toggled(collection(a, "users"));
    let mut harness = harness(state);
    let crm = db_row_label("crm", true);
    let end = drag_over(&mut harness, "users", &crm);
    // Pressing the row must not have narrowed the selection (a focus move would).
    assert!(harness.state().1.is_none());
    release(&mut harness, end);
    match dropped(harness.state_mut().1.take()).1 {
        SidebarAction::Drop { clipboard, target } => {
            let both = names(&["orders", "users"]);
            let expected = CopyClipboard::collections(a, "shop", both).unwrap();
            assert_eq!(*clipboard, expected);
            assert!(matches!(target, PasteTarget::Database { session, .. } if session == b));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn a_database_dropped_on_a_connection_name_is_a_drop_onto_that_connection() {
    let f = fixture();
    let (a, b) = (f.a, f.b);
    let mut harness = harness(f.state);
    let shop = db_row_label("shop", true);
    let end = drag_over(&mut harness, &shop, "Beta");
    release(&mut harness, end);
    match dropped(harness.state_mut().1.take()).1 {
        SidebarAction::Drop { clipboard, target } => {
            assert_eq!(*clipboard, CopyClipboard::database(a, "shop").unwrap());
            assert_eq!(target, PasteTarget::Connection { session: b });
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn a_target_that_refuses_the_drag_shows_why_and_a_release_does_nothing() {
    let mut harness = harness(fixture().state);
    let reason = I18n::english().t(paste_block_key(PasteBlock::CollectionsOntoCollection));
    let end = drag_over(&mut harness, "orders", "contacts");
    harness.get_by_label(&reason);
    release(&mut harness, end);
    assert!(harness.state_mut().1.take().is_none());
}

#[test]
fn a_target_that_takes_the_drag_shows_no_refusal() {
    let mut harness = harness(fixture().state);
    let crm = db_row_label("crm", true);
    drag_over(&mut harness, "orders", &crm);
    for key in [
        PasteBlock::CollectionsOntoCollection,
        PasteBlock::ReadOnlyTarget,
        PasteBlock::DatabaseOntoDatabase,
    ] {
        let text = I18n::english().t(paste_block_key(key));
        assert!(harness.query_by_label(&text).is_none(), "{text}");
    }
}

#[test]
fn a_plain_click_still_clicks_and_a_short_press_does_not_drag() {
    let mut harness = harness(fixture().state);
    harness.get_by_label("orders").click();
    harness.run();
    assert!(matches!(
        harness.state_mut().1.take(),
        Some(Input::Sidebar(SidebarAction::InSession { .. }))
    ));
}

#[test]
fn draggable_rows_keep_their_label_and_role() {
    let harness = harness(fixture().state);
    let role = egui::accesskit::Role::Button;
    harness.get_by_role_and_label(role, "orders");
    harness.get_by_role_and_label(role, &db_row_label("shop", true));
}

#[test]
fn paste_by_keyboard_and_menu_is_unaffected_by_dragging() {
    let f = fixture();
    let i18n = I18n::english();
    let mut state = f.state;
    state.clipboard = CopyClipboard::collections(f.a, "shop", names(&["orders"])).ok();
    let mut harness = harness(state);
    harness.get_by_label(&db_row_label("crm", true)).focus();
    harness.run();
    harness.state_mut().1 = None;
    harness
        .input_mut()
        .events
        .push(egui::Event::Paste(String::new()));
    harness.run();
    let (_, action) = dropped(harness.state_mut().1.take());
    assert!(matches!(
        action,
        SidebarAction::Paste(PasteTarget::Database { .. })
    ));
    harness
        .get_by_label(&db_row_label("crm", true))
        .click_secondary();
    harness.run();
    harness.state_mut().1 = None;
    harness.get_by_label(&i18n.t("copy.paste")).click();
    harness.run();
    let (_, action) = dropped(harness.state_mut().1.take());
    assert!(matches!(
        action,
        SidebarAction::Paste(PasteTarget::Database { .. })
    ));
}
