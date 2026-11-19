use super::TabStrip;
use crate::app::tabs::TabId;
use uuid::Uuid;

fn three() -> (TabStrip, [TabId; 3], Uuid) {
    let session = Uuid::new_v4();
    let strip = TabStrip::default();
    let (strip, a) = strip.opened_for(session, None);
    let (strip, b) = strip.opened_for(session, None);
    let (strip, c) = strip.opened_for(session, None);
    (strip, [a, b, c], session)
}

#[test]
fn a_sessions_first_tab_is_its_current_one() {
    let session = Uuid::new_v4();
    let (strip, first) = TabStrip::default().opened_for(session, None);
    let (strip, second) = strip.opened_for(session, None);
    assert_eq!(strip.current.get(&session), Some(&first));
    assert_eq!(strip.active, Some(first));
    assert_eq!(strip.of_session(session), vec![first, second]);
}

#[test]
fn a_tab_opens_after_the_tab_it_names() {
    let (strip, [a, b, c], session) = three();
    let (strip, d) = strip.opened_for(session, Some(a));
    assert_eq!(strip.order, vec![a, d, b, c]);
}

#[test]
fn closing_active_focuses_right_then_left() {
    let (strip, [a, b, c], _) = three();
    let strip = strip.activated(b).closed(b);
    assert_eq!(strip.active, Some(c), "the right neighbour");
    let strip = strip.closed(c);
    assert_eq!(strip.active, Some(a), "no right neighbour: the left one");
    let strip = strip.closed(a);
    assert_eq!(strip.active, None);
    assert!(strip.order.is_empty() && strip.owner.is_empty() && strip.mru.is_empty());
}

#[test]
fn closing_another_tab_keeps_the_active_one() {
    let (strip, [a, b, c], _) = three();
    let strip = strip.activated(c).closed(a);
    assert_eq!(strip.active, Some(c));
    assert_eq!(strip.order, vec![b, c]);
}

#[test]
fn closing_a_current_tab_leaves_its_session_without_one() {
    let (strip, [a, _, _], session) = three();
    let strip = strip.closed(a);
    assert_eq!(strip.current.get(&session), None);
}

#[test]
fn closing_an_unknown_tab_changes_nothing() {
    let (strip, _, _) = three();
    assert_eq!(strip.clone().closed(TabId(999)), strip);
}

#[test]
fn a_closed_sessions_tabs_all_go() {
    let (strip, [_, _, _], session) = three();
    let other = Uuid::new_v4();
    let (strip, keep) = strip.opened_for(other, None);
    let strip = strip.session_closed(session);
    assert_eq!(strip.order, vec![keep]);
    assert_eq!(strip.current.get(&other), Some(&keep));
    assert_eq!(strip.current.get(&session), None);
}

#[test]
fn mru_order() {
    let (strip, [a, b, c], _) = three();
    assert_eq!(
        strip.mru,
        vec![a, b, c],
        "opening does not make a tab recent"
    );
    let strip = strip.activated(c).activated(a);
    assert_eq!(strip.mru, vec![a, c, b]);
    assert_eq!(strip.active, Some(a));
    let strip = strip.activated(c);
    assert_eq!(strip.mru, vec![c, a, b]);
}

#[test]
fn activating_an_unknown_tab_changes_nothing() {
    let (strip, _, _) = three();
    assert_eq!(strip.clone().activated(TabId(999)), strip);
}

#[test]
fn moved_clamps() {
    let (strip, [a, b, c], _) = three();
    let strip = strip.moved(a, 99);
    assert_eq!(strip.order, vec![b, c, a]);
    let strip = strip.moved(a, 0);
    assert_eq!(strip.order, vec![a, b, c]);
    assert_eq!(strip.clone().moved(TabId(999), 0), strip);
}

#[test]
fn ids_never_repeat() {
    let session = Uuid::new_v4();
    let (strip, a) = TabStrip::default().opened_for(session, None);
    let strip = strip.closed(a);
    let (strip, b) = strip.opened_for(session, None);
    let strip = strip.closed(b);
    let (_, c) = strip.opened_for(session, None);
    assert!(a != b && b != c && a != c);
}
