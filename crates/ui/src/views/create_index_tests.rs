use super::*;

#[test]
fn nothing_in_the_way_means_on() {
    assert_eq!(CreateIndex::of(false, false, false, false), CreateIndex::On);
}

#[test]
fn each_cause_has_its_own_state() {
    assert_eq!(
        CreateIndex::of(true, false, false, false),
        CreateIndex::ReadOnly
    );
    assert_eq!(
        CreateIndex::of(false, true, false, false),
        CreateIndex::IndexWrite
    );
    assert_eq!(
        CreateIndex::of(false, false, true, false),
        CreateIndex::Prompt
    );
    assert_eq!(
        CreateIndex::of(false, false, false, true),
        CreateIndex::WriteWarning
    );
}

#[test]
fn read_only_wins_then_the_write_in_flight_then_the_warning() {
    assert_eq!(
        CreateIndex::of(true, true, true, true),
        CreateIndex::ReadOnly
    );
    assert_eq!(
        CreateIndex::of(false, true, true, true),
        CreateIndex::IndexWrite
    );
    assert_eq!(
        CreateIndex::of(false, false, true, true),
        CreateIndex::WriteWarning
    );
}

#[test]
fn every_off_state_has_a_distinct_reason_and_on_and_hidden_have_none() {
    let reasons: Vec<_> = [
        CreateIndex::ReadOnly,
        CreateIndex::IndexWrite,
        CreateIndex::Prompt,
        CreateIndex::WriteWarning,
    ]
    .map(|c| c.off_reason())
    .into_iter()
    .collect();
    assert!(reasons.iter().all(Option::is_some));
    let mut keys: Vec<_> = reasons.into_iter().flatten().collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), 4);
    assert_eq!(CreateIndex::On.off_reason(), None);
    assert_eq!(CreateIndex::Hidden.off_reason(), None);
}
