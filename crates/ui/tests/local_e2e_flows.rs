//! Click-driven UI tests: the REAL app and runtime, driven through the accessibility tree by
//! `egui_kittest`, against a real MongoDB. After each action both the screen and the database
//! (read through the driver) are asserted.
//!
//! Tier 1 (local server): needs only a MongoDB already running (`CURLYBRACE_IT_MONGODB_URI`,
//! default `mongodb://127.0.0.1:27017`); no docker. Every test writes to its own throwaway
//! `cbit_*` database and drops it. Run with `scripts/integration-tests.sh`. The flows that need
//! docker (`--auth`, stopping the server) are in `docker_e2e_failures.rs`.
mod support;

use bson::{Bson, doc};
use curlybrace_core::secrets::{SecretKind, SecretStore, secret_key};
use curlybrace_ui::app::persistence::{load_connections, save_connections};
use curlybrace_ui::viewmodel::id_preview;
use egui_kittest::kittest::Queryable;
use support::actions::*;
use support::db::people;
use support::fixture::Fixture;
use support::screen::shows_containing;
use support::wait::{SHORT, wait_until};

const PEOPLE: &str = "people";
const NAMES: [&str; 3] = ["Ada Lovelace", "Grace Hopper", "Alan Turing"];

/// The name of the enclosing test function (for the printed skip reason).
macro_rules! test_name {
    () => {{
        fn here() {}
        std::any::type_name_of_val(&here).trim_end_matches("::here")
    }};
}

macro_rules! fixture {
    ($ctor:ident) => {{
        let Some(f) = Fixture::$ctor(test_name!()) else {
            return;
        };
        f.db.insert(PEOPLE, people());
        f
    }};
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn add_a_connection_through_the_form_then_browse_it() {
    let mut f = fixture!(local_empty);
    let connect = f.labels.t("connections.connect");
    assert!(
        f.harness.query_all_by_label(&connect).next().is_none(),
        "no saved connection yet"
    );

    fill_key(&mut f, "connections.name", "Local test");
    let uri = f.server.uri();
    fill_key(&mut f, "connections.uri", &uri);
    click_key(&mut f, "common.save");

    wait_shows(&mut f, "Local test");
    let saved = load_connections(&f.config_path)
        .expect("connections file")
        .connections;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "Local test");
    assert_eq!(saved[0].uri, f.server.uri());
    assert!(
        f.secrets
            .get(&secret_key(saved[0].id, SecretKind::Password))
            .unwrap()
            .is_none(),
        "a URI without a password stores no secret"
    );

    browse(&mut f, PEOPLE);
    assert_names_on_screen(&f, &NAMES);
    let in_db: Vec<String> =
        f.db.all(PEOPLE)
            .iter()
            .map(|d| d.get_str("name").unwrap().to_string())
            .collect();
    assert_eq!(in_db, NAMES);
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn find_bar_filter_sort_projection_and_bad_input() {
    let mut f = fixture!(local);
    browse(&mut f, PEOPLE);
    let (filter, sort, projection) = (
        f.labels.t("findbar.filter"),
        f.labels.t("findbar.sort"),
        f.labels.t("findbar.projection"),
    );

    fill(&mut f, &filter, r#"{"active": true}"#);
    click_key(&mut f, "findbar.run");
    wait_gone(&mut f, "Alan Turing");
    assert_names_on_screen(&f, &["Ada Lovelace", "Grace Hopper"]);
    assert_eq!(f.db.count(PEOPLE), 3, "a find never changes the data");

    fill(&mut f, &sort, r#"{"seats": -1}"#);
    click_key(&mut f, "findbar.run");
    wait_until(&mut f.harness, "Grace sorted first", SHORT, |h| {
        appears_before(h, "Grace Hopper", "Ada Lovelace")
    });

    fill(&mut f, &projection, r#"{"name": 1}"#);
    click_key(&mut f, "findbar.run");
    wait_gone(&mut f, "pro");
    assert_names_on_screen(&f, &["Ada Lovelace", "Grace Hopper"]);
    assert!(
        f.harness.query_all_by_label("plan").next().is_none(),
        "projected away"
    );

    // Bad JSON: an error, the previous results stay, the typed text is kept.
    fill(&mut f, &filter, "{ nope");
    click_key(&mut f, "findbar.run");
    let invalid = f.labels.t("errors.invalid_json");
    wait_shows(&mut f, &invalid);
    assert_names_on_screen(&f, &["Ada Lovelace", "Grace Hopper"]);
    assert_eq!(text_of(&f.harness, &filter), "{ nope");
    assert_eq!(f.db.count(PEOPLE), 3);
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn switching_tree_table_and_json_changes_the_content() {
    let mut f = fixture!(local);
    browse(&mut f, PEOPLE);
    let int32 = f.labels.t("bsontype.int32");
    assert_names_on_screen(&f, &["Ada Lovelace"]);
    assert!(
        f.harness.query_all_by_label(&int32).next().is_some(),
        "tree shows type tags"
    );

    click_key(&mut f, "view.table");
    wait_until(&mut f.harness, "table columns", SHORT, |h| {
        h.query_all_by_label("seats").next().is_some()
            && h.query_all_by_label(&int32).next().is_none()
    });
    assert_names_on_screen(&f, &["Ada Lovelace", "Grace Hopper", "Alan Turing", "40"]);

    click_key(&mut f, "view.json");
    wait_until(&mut f.harness, "json lines", SHORT, |h| {
        shows_containing(h, "\"name\": \"Ada Lovelace\"")
    });
    assert!(
        f.harness.query_all_by_label("seats").next().is_none(),
        "no table header in JSON"
    );

    click_key(&mut f, "view.tree");
    wait_until(&mut f.harness, "tree again", SHORT, |h| {
        h.query_all_by_label(&int32).next().is_some()
    });
    assert!(!shows_containing(&f.harness, "\"name\": \"Ada Lovelace\""));
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn edit_a_document_through_the_editor_saves_and_refreshes() {
    let mut f = fixture!(local);
    browse(&mut f, PEOPLE);
    let first = header(&f, 1);

    choose_from_menu(&mut f, &first, "doc.edit");
    let title = f.labels.t("editor.title_edit");
    wait_shows(&mut f, &title);
    let original = text_of(&f.harness, &title);
    assert!(
        original.contains("Ada Lovelace"),
        "editor shows the document: {original}"
    );
    fill(
        &mut f,
        &title,
        &original.replace("Ada Lovelace", "Ada King"),
    );
    click_key(&mut f, "editor.save");

    wait_until(&mut f.harness, "database updated", SHORT, |_| {
        f.db.find_one(PEOPLE, doc! {"_id": 1_i32})
            .is_some_and(|d| d.get_str("name") == Ok("Ada King"))
    });
    wait_shows(&mut f, "Ada King");
    wait_gone(&mut f, "Ada Lovelace");
    wait_gone(&mut f, &title);
    let updated = f.db.find_one(PEOPLE, doc! {"_id": 1_i32}).unwrap();
    assert_eq!(
        updated.get_i32("seats"),
        Ok(12),
        "other fields keep value and type"
    );
    assert_eq!(f.db.count(PEOPLE), 3);
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn clone_a_document_adds_a_new_one() {
    let mut f = fixture!(local);
    browse(&mut f, PEOPLE);
    let first = header(&f, 1);

    choose_from_menu(&mut f, &first, "doc.clone");
    let fourth = header(&f, 4);
    wait_shows(&mut f, &fourth);

    assert_eq!(f.db.count(PEOPLE), 4);
    let copies: Vec<_> =
        f.db.all(PEOPLE)
            .into_iter()
            .filter(|d| d.get_str("name") == Ok("Ada Lovelace"))
            .collect();
    assert_eq!(copies.len(), 2, "the original and its clone");
    assert!(
        copies
            .iter()
            .any(|d| matches!(d.get("_id"), Some(Bson::ObjectId(_)))),
        "the clone has a fresh _id"
    );
    assert_eq!(f.harness.query_all_by_label("Ada Lovelace").count(), 2);
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn delete_asks_for_confirmation_cancel_keeps_confirm_removes() {
    let mut f = fixture!(local);
    browse(&mut f, PEOPLE);
    let first = header(&f, 1);
    let id = id_preview(&Bson::Int32(1));
    let ask = f.labels.t_with(
        "doc.confirm_delete",
        &[("id", &id), ("connection", "local")],
    );

    choose_from_menu(&mut f, &first, "doc.delete");
    wait_shows(&mut f, &ask);
    assert_eq!(
        f.db.count(PEOPLE),
        3,
        "nothing is deleted before confirming"
    );
    click_key(&mut f, "common.cancel");
    wait_gone(&mut f, &ask);
    assert_eq!(f.db.count(PEOPLE), 3);
    assert_names_on_screen(&f, &["Ada Lovelace"]);

    choose_from_menu(&mut f, &first, "doc.delete");
    wait_shows(&mut f, &ask);
    click_key(&mut f, "doc.delete");
    wait_gone(&mut f, "Ada Lovelace");
    assert_eq!(f.db.count(PEOPLE), 2);
    assert!(f.db.find_one(PEOPLE, doc! {"_id": 1_i32}).is_none());
    assert_names_on_screen(&f, &["Grace Hopper", "Alan Turing"]);
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_projected_page_offers_delete_but_not_edit_or_clone() {
    let mut f = fixture!(local);
    browse(&mut f, PEOPLE);
    let projection = f.labels.t("findbar.projection");
    let first = header(&f, 1);
    let all = ["doc.edit", "doc.clone", "doc.delete"];

    assert_eq!(
        menu_entries(&mut f, &first, &all),
        all,
        "a full page offers everything"
    );

    fill(&mut f, &projection, r#"{"name": 1}"#);
    click_key(&mut f, "findbar.run");
    wait_gone(&mut f, "pro");
    assert_eq!(menu_entries(&mut f, &first, &all), ["doc.delete"]);
    assert_eq!(f.db.all(PEOPLE), people(), "nothing was written");
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn deleting_a_saved_connection_asks_first_and_removes_its_secrets() {
    let mut f = fixture!(local);
    let id = f.saved_id();
    let key = secret_key(id, SecretKind::Password);
    f.secrets.set(&key, "saved-pw").unwrap();
    let list = f.harness.state().state().conn.connections.clone();
    save_connections(&f.config_path, &list).unwrap();
    let ask = f
        .labels
        .t_with("connections.confirm_delete", &[("name", "local")]);
    let delete = f.labels.t("common.delete");

    f.click_text(&delete);
    wait_shows(&mut f, &ask);
    click_key(&mut f, "common.cancel");
    wait_gone(&mut f, &ask);
    assert_eq!(
        f.harness.state().state().conn.connections.len(),
        1,
        "cancel keeps it"
    );
    assert_eq!(f.secrets.get(&key).unwrap().as_deref(), Some("saved-pw"));
    assert_eq!(
        load_connections(&f.config_path).unwrap().connections.len(),
        1
    );

    f.click_text(&delete);
    wait_shows(&mut f, &ask);
    // The row button and the modal button share a label: take the one under the question.
    click_button_below(&mut f, &delete, &ask);
    wait_until(&mut f.harness, "connection removed", SHORT, |h| {
        h.state().state().conn.connections.is_empty()
    });
    assert!(
        f.secrets.get(&key).unwrap().is_none(),
        "stored secret removed"
    );
    assert!(
        load_connections(&f.config_path)
            .unwrap()
            .connections
            .is_empty()
    );
}
