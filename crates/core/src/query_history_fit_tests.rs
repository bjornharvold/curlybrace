//! Making room in the file: dropping the oldest history in one pass, and favourites that keep
//! the id the person saw.
use super::*;

fn text(filter: &str) -> FindText {
    FindText {
        filter: filter.into(),
        ..FindText::default()
    }
}

fn conn_of(i: usize) -> Uuid {
    Uuid::from_u128(i as u128 + 1)
}

fn many(n: usize) -> QueryHistory {
    (0..n).fold(QueryHistory::default(), |s, i| {
        s.recorded(
            conn_of(i % 3),
            "shop",
            &format!("c{}", i % 4),
            text(&format!("{{\"n\": {i}}}")),
            i as i64,
        )
        .unwrap()
    })
}

fn kept_times(s: &QueryHistory) -> Vec<i64> {
    let mut times = Vec::new();
    for i in 0..3 {
        for c in 0..4 {
            let list = s.history(conn_of(i), "shop", &format!("c{c}"));
            times.extend(list.iter().map(|e| e.ran_unix));
        }
    }
    times.sort_unstable();
    times
}

#[test]
fn dropping_by_bytes_removes_the_oldest_across_namespaces_in_one_step() {
    let s = many(40);
    let one = serde_json::to_vec_pretty(&s.history(conn_of(0), "shop", "c0")[0])
        .unwrap()
        .len();
    let trimmed = s.oldest_history_dropped_by(one * 10).unwrap();
    let dropped = 40 - trimmed.history_len();
    // Each entry counts a little more than its own print (its indentation in the file).
    assert!((5..=10).contains(&dropped), "about ten entries: {dropped}");
    // Only the oldest went: what is kept is exactly the newest run of times.
    let kept = kept_times(&trimmed);
    let expected: Vec<i64> = (dropped as i64..40).collect();
    assert_eq!(kept, expected);
}

#[test]
fn dropping_one_byte_drops_one_entry_and_nothing_means_none() {
    let s = many(10);
    assert_eq!(s.oldest_history_dropped_by(1).unwrap().history_len(), 9);
    assert_eq!(s.oldest_history_dropped().unwrap().history_len(), 9);
    assert_eq!(QueryHistory::default().oldest_history_dropped_by(10), None);
}

#[test]
fn dropping_more_than_there_is_empties_the_history_and_keeps_favourites() {
    let s = many(10)
        .starred(conn_of(0), "shop", "c0", "keep", text("a"), 1)
        .unwrap();
    let trimmed = s.oldest_history_dropped_by(usize::MAX).unwrap();
    assert_eq!(trimmed.history_len(), 0);
    assert_eq!(trimmed.favourites(conn_of(0)).len(), 1);
}

#[test]
fn an_id_that_is_already_a_favourite_is_refused() {
    let id = Uuid::from_u128(9);
    let c = conn_of(0);
    let s = QueryHistory::default()
        .starred_with_id(id, c, "shop", "c0", "one", text("a"), 1)
        .unwrap();
    assert_eq!(
        s.starred_with_id(id, c, "shop", "c1", "two", text("b"), 2),
        Err(SavedError::NameTaken)
    );
}

#[test]
fn the_same_favourite_again_is_already_there() {
    let id = Uuid::from_u128(9);
    let c = conn_of(0);
    let s = QueryHistory::default()
        .starred_with_id(id, c, "shop", "c0", "one", text("a"), 1)
        .unwrap();
    assert_eq!(
        s.starred_with_id(id, c, "shop", "c0", " ONE ", text("a"), 5),
        Ok(s.clone())
    );
}

#[test]
fn the_flag_alone_can_be_set_without_erasing() {
    let s = many(5);
    assert_eq!(s.flag_set(false).history_len(), 5);
    assert!(!s.flag_set(false).remembering());
}

#[test]
fn switching_on_keeps_the_history_that_is_there() {
    let s = many(5).with_remembering(true);
    assert_eq!(s.history_len(), 5);
    assert_eq!(many(5).with_remembering(false).history_len(), 0);
}

#[test]
fn a_favourite_can_be_saved_under_an_id_that_stays_the_same() {
    let id = Uuid::from_u128(4242);
    let c = conn_of(0);
    let s = QueryHistory::default()
        .starred_with_id(id, c, "shop", "c0", "n", text("a"), 1)
        .unwrap();
    assert_eq!(s.favourites(c)[0].id, id);
    let other = QueryHistory::default()
        .starred(c, "shop", "c0", "n", text("a"), 1)
        .unwrap();
    assert_ne!(other.favourites(c)[0].id, id);
}
