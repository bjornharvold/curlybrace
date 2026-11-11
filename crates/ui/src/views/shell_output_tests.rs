use super::*;

#[test]
fn each_session_scrolls_its_own_output() {
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    assert_ne!(scroll_salt(a), scroll_salt(b));
    assert_eq!(scroll_salt(a), scroll_salt(a));
}
