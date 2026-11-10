//! The list of saved undo copies in the accesskit tree: every button is named for the copy it
//! acts on, Delete asks first, and the list says where the copies are. Fast, offline, no GPU.
use curlybrace_core::bulk::BulkRunId;
use curlybrace_core::snapshot::{SnapshotInfo, SnapshotOp};
use curlybrace_core::store::Namespace;
use curlybrace_ui::I18n;
use curlybrace_ui::app::saved_copies::{CopiesAction, SavedCopies};
use curlybrace_ui::views::saved_copies::show_saved_copies;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::path::PathBuf;

const WHEN: &str = "2023-11-14 22:13 UTC";

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn tw(key: &str, params: &[(&str, &str)]) -> String {
    I18n::english().t_with(key, params)
}

fn run(n: u128) -> BulkRunId {
    BulkRunId(uuid::Uuid::from_u128(n))
}

fn info(n: u128, collection: &str, op: SnapshotOp) -> SnapshotInfo {
    SnapshotInfo {
        run: run(n),
        ns: Namespace::new("shop", collection),
        op,
        created_unix: 1_700_000_000,
        bytes: 2048,
    }
}

struct Frame {
    copies: SavedCopies,
    seen: Vec<CopiesAction>,
}

fn copies(list: Option<Vec<SnapshotInfo>>, asking: Option<BulkRunId>) -> SavedCopies {
    SavedCopies {
        list,
        awaiting: None,
        asking,
    }
}

fn open(copies: SavedCopies) -> Harness<'static, Frame> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 800.0))
        .build_state(
            |ctx, f: &mut Frame| {
                let folder = PathBuf::from("/snapshots");
                let action = show_saved_copies(ctx, &f.copies, Some(&folder), &I18n::english());
                f.seen.extend(action);
            },
            Frame {
                copies,
                seen: Vec::new(),
            },
        );
    h.run_steps(3);
    h
}

fn two() -> Vec<SnapshotInfo> {
    vec![
        info(1, "orders", SnapshotOp::Delete),
        info(2, "users", SnapshotOp::Update),
    ]
}

#[test]
fn the_list_names_the_folder_and_each_copy() {
    let h = open(copies(Some(two()), None));
    h.get_by_label(&t("copies.title"));
    let days = (curlybrace_core::snapshot::RETENTION_SECS / 86_400).to_string();
    let intro = tw("copies.intro", &[("folder", "/snapshots"), ("days", &days)]);
    h.get_by_label(&intro);
    let row = tw(
        "copies.row",
        &[
            ("ns", "shop.orders"),
            ("op", &t("copies.op_delete")),
            ("when", WHEN),
            ("size", &tw("indexes.size_kib", &[("n", "2.0")])),
        ],
    );
    h.get_by_label(&row);
}

#[test]
fn each_delete_button_is_named_for_its_copy() {
    let h = open(copies(Some(two()), None));
    for ns in ["shop.orders", "shop.users"] {
        let name = tw("copies.delete_named", &[("ns", ns), ("when", WHEN)]);
        h.get_by_role_and_label(Role::Button, &name);
    }
}

#[test]
fn delete_asks_before_it_removes_anything() {
    let mut h = open(copies(Some(two()), None));
    let name = tw(
        "copies.delete_named",
        &[("ns", "shop.orders"), ("when", WHEN)],
    );
    h.get_by_role_and_label(Role::Button, &name).click();
    h.run_steps(3);
    assert_eq!(h.state().seen, vec![CopiesAction::Ask(run(1))]);

    let mut h = open(copies(Some(two()), Some(run(1))));
    assert!(h.query_all_by_label(&t("copies.confirm")).next().is_some());
    h.get_by_role_and_label(Role::Button, &t("copies.confirm_delete"))
        .click();
    h.run_steps(3);
    assert_eq!(h.state().seen, vec![CopiesAction::Delete(run(1))]);
}

#[test]
fn keeping_the_copy_is_one_click() {
    let mut h = open(copies(Some(two()), Some(run(2))));
    h.get_by_role_and_label(Role::Button, &t("copies.keep"))
        .click();
    h.run_steps(3);
    assert_eq!(h.state().seen, vec![CopiesAction::Keep]);
}

#[test]
fn nothing_can_be_asked_while_an_answer_is_awaited() {
    let busy = SavedCopies {
        awaiting: Some((1, None)),
        ..copies(Some(two()), None)
    };
    let h = open(busy);
    let name = tw(
        "copies.delete_named",
        &[("ns", "shop.orders"), ("when", WHEN)],
    );
    let node = h.get_by_role_and_label(Role::Button, &name);
    assert!(node.accesskit_node().is_disabled());
}

#[test]
fn loading_and_empty_say_so_and_close_is_always_there() {
    let h = open(copies(None, None));
    h.get_by_label(&t("copies.loading"));
    h.get_by_role_and_label(Role::Button, &t("common.close"));
    let mut h = open(copies(Some(Vec::new()), None));
    h.get_by_label(&t("copies.empty"));
    h.get_by_role_and_label(Role::Button, &t("common.close"))
        .click();
    h.run_steps(3);
    assert_eq!(h.state().seen, vec![CopiesAction::Close]);
}
