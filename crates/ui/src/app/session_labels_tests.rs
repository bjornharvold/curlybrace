//! Open sessions that share a saved name are told apart, by one function, wherever shown.
use crate::I18n;
use crate::app::banner::banner_prefix;
use crate::app::reducer::{Input, update};
use crate::app::session::session_display_name;
use crate::app::state::{AppState, Banner};
use crate::viewmodel::ConnectionBadge;
use crate::views::sidebar_roots::show_roots;
use curlybrace_core::connection::{ConnectionConfig, SshAuth, SshConfig};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Event;
use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use uuid::Uuid;

fn cfg(name: &str, uri: &str, ssh_host: &str) -> ConnectionConfig {
    let mut c = ConnectionConfig::new(name, uri);
    c.ssh = Some(SshConfig {
        host: ssh_host.into(),
        port: 22,
        username: "me".into(),
        auth: SshAuth::Password,
    });
    c
}

fn with_id(mut c: ConnectionConfig, id: &str) -> ConnectionConfig {
    c.id = Uuid::parse_str(id).unwrap();
    c
}

fn i18n() -> I18n {
    I18n::english()
}

fn opened(state: AppState, id: Uuid) -> AppState {
    let (s, _) = update(state, Input::Connect(id));
    let epoch = s.epoch_counter;
    update(
        s,
        Input::Event(Event::Connected {
            session: id,
            epoch,
            topology: None,
        }),
    )
    .0
}

fn connecting(state: AppState, id: Uuid) -> AppState {
    update(state, Input::Connect(id)).0
}

fn root_names(s: &AppState) -> Vec<String> {
    s.roots(&i18n()).into_iter().map(|r| r.name).collect()
}

fn two_prod() -> (AppState, Uuid, Uuid) {
    let a = cfg("Prod", "mongodb://hostA:27017", "bastion-a");
    let b = cfg("Prod", "mongodb://hostB:27017", "bastion-b");
    let (ia, ib) = (a.id, b.id);
    (AppState::new(vec![a, b]), ia, ib)
}

#[test]
fn unique_names_are_shown_unchanged_everywhere() {
    let a = cfg("Prod", "mongodb://hostA:27017", "bastion-a");
    let b = cfg("Stage", "mongodb://hostB:27017", "bastion-b");
    let (ia, ib) = (a.id, b.id);
    let s = opened(opened(AppState::new(vec![a, b]), ia), ib);
    assert_eq!(root_names(&s), vec!["Prod", "Stage"]);
    assert_eq!(s.active_connection_label(&i18n()), "Stage");
    assert_eq!(session_display_name(&s, ia, &i18n()).unwrap(), "Prod");
    let c = cfg("Solo", "mongodb://h:1", "bastion");
    let ic = c.id;
    let s = connecting(AppState::new(vec![c]), ic);
    assert_eq!(
        s.connecting_name_for_host("bastion", &i18n()).unwrap(),
        "Solo"
    );
}

#[test]
fn two_open_sessions_with_one_name_get_their_servers_appended() {
    let (s, a, b) = two_prod();
    let s = opened(opened(s, a), b);
    assert_eq!(
        root_names(&s),
        vec!["Prod (hostA:27017)", "Prod (hostB:27017)"]
    );
    assert_eq!(s.active_connection_label(&i18n()), "Prod (hostB:27017)");
    assert_eq!(
        session_display_name(&s, a, &i18n()).unwrap(),
        "Prod (hostA:27017)"
    );
}

#[test]
fn the_host_key_name_and_banner_prefix_use_the_disambiguated_label() {
    let (s, a, b) = two_prod();
    let s = connecting(connecting(s, a), b);
    assert_eq!(
        s.connecting_name_for_host("bastion-a", &i18n()).unwrap(),
        "Prod (hostA:27017)"
    );
    assert_eq!(
        s.connecting_name_for_host("bastion-b", &i18n()).unwrap(),
        "Prod (hostB:27017)"
    );
    let owned = AppState {
        banner: Some(Banner::Failure {
            context: "find",
            error: CoreError::NotFound,
        }),
        banner_session: Some(b),
        ..s
    };
    let prefix = banner_prefix(&owned, &i18n()).unwrap();
    assert!(prefix.contains("Prod (hostB:27017)"), "{prefix}");
}

#[test]
fn closing_one_restores_the_plain_name_of_the_other() {
    let (s, a, b) = two_prod();
    let s = opened(opened(s, a), b);
    let (s, _) = update(s, Input::Disconnect(b));
    assert_eq!(root_names(&s), vec!["Prod"]);
    assert_eq!(s.active_connection_label(&i18n()), "Prod");
    assert_eq!(session_display_name(&s, a, &i18n()).unwrap(), "Prod");
}

#[test]
fn identical_servers_add_the_first_four_id_characters() {
    let a = with_id(
        cfg("Prod", "mongodb://h:27017", "bastion"),
        "abcd0000-0000-0000-0000-000000000001",
    );
    let b = with_id(
        cfg("Prod", "mongodb://h:27017", "bastion"),
        "ef010000-0000-0000-0000-000000000002",
    );
    let (ia, ib) = (a.id, b.id);
    let s = opened(opened(AppState::new(vec![a, b]), ia), ib);
    assert_eq!(
        root_names(&s),
        vec!["Prod (h:27017) #abcd", "Prod (h:27017) #ef01"]
    );
}

#[test]
fn a_third_session_with_other_servers_stays_without_an_id() {
    let mk = |uri: &str, id: &str| with_id(cfg("Prod", uri, "b"), id);
    let cfgs = vec![
        mk("mongodb://h:1", "aaaa0000-0000-0000-0000-000000000001"),
        mk("mongodb://h:1", "bbbb0000-0000-0000-0000-000000000002"),
        mk("mongodb://other:1", "cccc0000-0000-0000-0000-000000000003"),
    ];
    let ids: Vec<Uuid> = cfgs.iter().map(|c| c.id).collect();
    let s = ids.iter().fold(AppState::new(cfgs), |s, id| opened(s, *id));
    assert_eq!(
        root_names(&s),
        vec!["Prod (h:1) #aaaa", "Prod (h:1) #bbbb", "Prod (other:1)"]
    );
}

#[test]
fn a_saved_but_closed_connection_with_the_same_name_adds_no_suffix() {
    let (s, a, _b) = two_prod();
    let s = opened(s, a);
    assert_eq!(root_names(&s), vec!["Prod"]);
    assert_eq!(s.active_connection_label(&i18n()), "Prod");
}

#[test]
fn the_saved_names_are_never_changed() {
    let (s, a, b) = two_prod();
    let s = opened(opened(s, a), b);
    assert!(s.conn.connections.iter().all(|c| c.name == "Prod"));
}

#[test]
fn a_deleted_connection_has_no_label_and_the_header_falls_back() {
    let (s, a, _) = two_prod();
    let mut s = opened(s, a);
    s.conn.connections.clear();
    assert_eq!(session_display_name(&s, a, &i18n()), None);
    assert_eq!(
        s.active_connection_label(&i18n()),
        i18n().t("doc.connection_unknown")
    );
}

#[test]
fn the_toggle_and_disconnect_controls_of_the_two_roots_have_distinct_names() {
    let (s, a, b) = two_prod();
    let s = opened(opened(s, a), b);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(360.0, 400.0))
        .build_ui_state(
            |ui, state: &mut AppState| {
                let _ = show_roots(ui, state, &I18n::english());
            },
            s,
        );
    harness.run();
    let i = i18n();
    for name in ["Prod (hostA:27017)", "Prod (hostB:27017)"] {
        for key in ["sidebar.toggle_named", "sidebar.disconnect_named"] {
            let label = i.t_with(key, &[("name", name)]);
            assert_eq!(harness.get_all_by_label(&label).count(), 1, "{label}");
        }
    }
}

fn two_replica_sets() -> (AppState, Uuid, Uuid) {
    let a = cfg("Prod", "mongodb://h1:1,h2:2", "bastion-a");
    let b = cfg("Prod", "mongodb://h3:3,h4:4", "bastion-b");
    let (ia, ib) = (a.id, b.id);
    (opened(opened(AppState::new(vec![a, b]), ia), ib), ia, ib)
}

#[test]
fn the_servers_in_a_label_use_the_translated_list_separator() {
    let (s, a, _) = two_replica_sets();
    assert_eq!(
        session_display_name(&s, a, &i18n()).unwrap(),
        "Prod (h1:1, h2:2)"
    );
    let en = include_str!("../../locales/en.json");
    let custom = en.replace(
        "\"list_separator\": \", \"",
        "\"list_separator\": \" \u{b7} \"",
    );
    assert_ne!(custom, en, "the separator key must exist in en.json");
    let other = I18n::from_json(&custom).unwrap();
    assert_eq!(
        session_display_name(&s, a, &other).unwrap(),
        "Prod (h1:1 \u{b7} h2:2)"
    );
}

#[test]
fn root_carries_color_and_production_of_its_connection() {
    use curlybrace_core::connection::{ConnectionColor, SavedColor};
    let mut tagged = cfg("Prod", "mongodb://hostA:27017", "bastion-a");
    tagged.color = Some(SavedColor::Known(ConnectionColor::Red));
    tagged.production = true;
    let plain = cfg("Dev", "mongodb://hostB:27017", "bastion-b");
    let (it, ip) = (tagged.id, plain.id);
    let s = opened(opened(AppState::new(vec![tagged, plain]), it), ip);
    let roots = s.roots(&i18n());
    assert_eq!(
        roots[0].badge,
        ConnectionBadge::new(Some(ConnectionColor::Red), true)
    );
    assert_eq!(roots[0].badge, s.session_badge(it));
    assert_eq!(roots[1].badge, ConnectionBadge::default());
    assert_eq!(roots[1].badge, s.session_badge(ip));
}

#[test]
fn active_badge_follows_the_active_connection() {
    use crate::viewmodel::ConnectionBadge;
    use curlybrace_core::connection::{ConnectionColor, SavedColor};
    let mut tagged = cfg("Prod", "mongodb://hostA:27017", "bastion-a");
    tagged.color = Some(SavedColor::Known(ConnectionColor::Blue));
    tagged.production = true;
    let id = tagged.id;
    let s = opened(AppState::new(vec![tagged]), id);
    assert_eq!(
        s.active_badge(),
        ConnectionBadge::new(Some(ConnectionColor::Blue), true)
    );
}

#[test]
fn embedded_session_has_no_badge() {
    use crate::viewmodel::ConnectionBadge;
    assert_eq!(
        AppState::default().active_badge(),
        ConnectionBadge::default()
    );
}

#[test]
fn an_unrecognised_colour_shows_as_no_colour_on_the_root_and_the_header() {
    use crate::viewmodel::ConnectionBadge;
    use curlybrace_core::connection::SavedColor;
    let mut odd = cfg("Prod", "mongodb://hostA:27017", "bastion-a");
    odd.color = Some(SavedColor::Unrecognised("future-shade".into()));
    odd.production = true;
    let id = odd.id;
    let s = opened(AppState::new(vec![odd]), id);
    let expected = ConnectionBadge::new(None, true);
    assert_eq!(s.roots(&i18n())[0].badge, expected);
    assert_eq!(s.session_badge(id), expected);
    assert_eq!(s.active_badge(), expected);
}
