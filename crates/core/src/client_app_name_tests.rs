use crate::client::{MAX_APP_NAME_BYTES, build_client_options, with_own_app_name};
use crate::connection::ConnectionConfig;
use crate::profile::{OWN_APP_NAME, ProfileEntry, is_own_traffic};
use bson::doc;
use mongodb::options::ClientOptions;

fn options_with(app_name: Option<&str>) -> ClientOptions {
    let mut o = ClientOptions::default();
    o.app_name = app_name.map(str::to_owned);
    o
}

fn marked(app_name: Option<&str>) -> String {
    with_own_app_name(options_with(app_name))
        .app_name
        .expect("the marker is always set")
}

/// Round trip through the entry classifier the profiler poller uses.
fn assert_classified_as_ours(app_name: &str) {
    let entry = ProfileEntry {
        ts_millis: 0,
        ts_secs: 0,
        ns: "shop.orders".into(),
        op: "query".into(),
        millis: 1,
        plan_summary: "COLLSCAN".into(),
        keys_examined: 0,
        docs_examined: 1,
        returned: 1,
        has_sort_stage: false,
        app_name: Some(app_name.to_owned()),
        command: doc! { "find": "orders" },
    };
    assert!(is_own_traffic(&entry), "{app_name:?}");
}

#[test]
fn sets_the_marker_when_absent() {
    let v = marked(None);
    assert_eq!(v, "curlybrace");
    assert_classified_as_ours(&v);
}

#[test]
fn appends_the_marker_to_a_user_supplied_app_name() {
    let v = marked(Some("reports"));
    assert_eq!(v, "reports curlybrace");
    assert_classified_as_ours(&v);
}

#[test]
fn empty_user_app_name_gets_just_the_marker() {
    let v = marked(Some(""));
    assert_eq!(v, "curlybrace");
    assert_classified_as_ours(&v);
}

#[test]
fn keeps_the_whole_value_within_128_bytes() {
    assert_eq!(MAX_APP_NAME_BYTES, 128);
    let long_ascii = "a".repeat(500);
    // 3-byte chars and 4-byte chars, so a naive byte cut would split a char.
    let multi3 = "€".repeat(100);
    let multi4 = "😀".repeat(100);
    let exact_fit = "b".repeat(MAX_APP_NAME_BYTES - OWN_APP_NAME.len() - 1);
    let one_over = "b".repeat(MAX_APP_NAME_BYTES - OWN_APP_NAME.len());
    for user in [long_ascii, multi3, multi4, exact_fit.clone(), one_over] {
        let v = marked(Some(&user));
        assert!(v.len() <= MAX_APP_NAME_BYTES, "{} bytes", v.len());
        assert!(v.ends_with(" curlybrace"), "{v:?}");
        assert!(user.starts_with(v.trim_end_matches(" curlybrace")));
        assert_classified_as_ours(&v);
    }
    // The exact fit is not truncated at all.
    assert_eq!(marked(Some(&exact_fit)).len(), MAX_APP_NAME_BYTES);
    assert_eq!(marked(Some(&exact_fit)), format!("{exact_fit} curlybrace"));
}

#[test]
fn does_not_double_append_when_already_marked() {
    for already in ["curlybrace", "bob curlybrace"] {
        assert_eq!(marked(Some(already)), already);
    }
    // Not ours by the classifier's rule, so these do get the marker.
    assert_eq!(marked(Some("curlybrace-2")), "curlybrace-2 curlybrace");
    assert_eq!(marked(Some("mycurlybrace")), "mycurlybrace curlybrace");
    assert_eq!(marked(Some("x curlybrace ")), "x curlybrace  curlybrace");
}

#[test]
fn marking_twice_is_the_same_as_marking_once() {
    let once = with_own_app_name(options_with(Some("bob")));
    let twice = with_own_app_name(once.clone());
    assert_eq!(once.app_name, twice.app_name);
}

#[tokio::test]
async fn uri_with_app_name_bob_at_laptop_round_trips() {
    let o = ClientOptions::parse("mongodb://h/?appName=bob@laptop")
        .await
        .unwrap();
    let v = with_own_app_name(o).app_name.unwrap();
    assert_eq!(v, "bob@laptop curlybrace");
    assert_classified_as_ours(&v);
}

#[tokio::test]
async fn uri_app_name_exactly_the_marker_is_unchanged() {
    let o = ClientOptions::parse("mongodb://h/?appName=curlybrace")
        .await
        .unwrap();
    assert_eq!(with_own_app_name(o).app_name.as_deref(), Some("curlybrace"));
}

#[tokio::test]
async fn build_client_options_applies_the_marker() {
    let plain = ConnectionConfig::new("a", "mongodb://localhost:27017");
    let o = build_client_options(&plain, None, None).await.unwrap();
    assert_eq!(o.app_name.as_deref(), Some("curlybrace"));

    let named = ConnectionConfig::new("b", "mongodb://localhost:27017/?appName=bob@laptop");
    let o = build_client_options(&named, None, None).await.unwrap();
    assert_eq!(o.app_name.as_deref(), Some("bob@laptop curlybrace"));
}
