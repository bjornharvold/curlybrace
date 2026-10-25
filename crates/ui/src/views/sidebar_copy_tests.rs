use super::*;

fn mods(command: bool, shift: bool) -> egui::Modifiers {
    egui::Modifiers {
        command,
        shift,
        ..Default::default()
    }
}

#[test]
fn a_modified_click_changes_the_selection_and_a_plain_click_does_not() {
    assert_eq!(click_mode(&mods(false, false)), None);
    assert_eq!(click_mode(&mods(true, false)), Some(SelectMode::Toggle));
    assert_eq!(click_mode(&mods(false, true)), Some(SelectMode::Extend));
    assert_eq!(click_mode(&mods(true, true)), Some(SelectMode::Toggle));
}

#[test]
fn keyboard_focus_selects_the_row_extends_with_shift_and_keeps_with_command() {
    assert_eq!(
        focus_mode(&mods(false, false), false),
        Some(SelectMode::Single)
    );
    assert_eq!(
        focus_mode(&mods(false, true), false),
        Some(SelectMode::Extend)
    );
    assert_eq!(focus_mode(&mods(true, false), false), None);
}

#[test]
fn shift_tab_selects_the_row_alone() {
    assert_eq!(
        focus_mode(&mods(false, true), true),
        Some(SelectMode::Single)
    );
}

fn orders_row(session: Uuid) -> NodeRef {
    NodeRef::Collection {
        session,
        ns: Namespace::new("shop", "orders"),
    }
}

#[test]
fn a_collection_row_menu_and_keyboard_paste_share_one_target() {
    let session = Uuid::from_u128(1);
    let collections =
        crate::viewmodel::CopyClipboard::collections(session, "shop", vec!["users".to_string()])
            .unwrap();
    let state = AppState {
        clipboard: Some(collections),
        ..AppState::default()
    };
    let ctx = TreeCtx {
        state: &state,
        session,
    };
    // Collections go onto the database of the row, in the menu as on Cmd+V.
    assert_eq!(
        row_paste_target(&ctx, &orders_row(session)),
        PasteTarget::Database {
            session,
            db: "shop".into()
        }
    );
    // Documents still go onto the collection itself.
    let documents = crate::viewmodel::CopyClipboard::Documents {
        session,
        ns: Namespace::new("shop", "users"),
        selection: crate::viewmodel::DocSelection::Query {
            filter: bson::doc! {},
            sort: None,
        },
    };
    let state = AppState {
        clipboard: Some(documents),
        ..AppState::default()
    };
    let ctx = TreeCtx {
        state: &state,
        session,
    };
    assert_eq!(
        row_paste_target(&ctx, &orders_row(session)),
        PasteTarget::Collection {
            session,
            ns: Namespace::new("shop", "orders")
        }
    );
}
