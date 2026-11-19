use super::*;

fn only(f: impl FnOnce(&mut CreateBlockers)) -> CreateIndex {
    let mut blockers = CreateBlockers::default();
    f(&mut blockers);
    CreateIndex::of(blockers)
}

#[test]
fn nothing_in_the_way_means_on() {
    assert_eq!(CreateIndex::of(CreateBlockers::default()), CreateIndex::On);
    assert_eq!(CreateIndex::read_only(false), CreateIndex::On);
}

#[test]
fn each_cause_has_its_own_state() {
    assert_eq!(only(|b| b.read_only = true), CreateIndex::ReadOnly);
    assert_eq!(only(|b| b.index_write = true), CreateIndex::IndexWrite);
    assert_eq!(only(|b| b.prompt = true), CreateIndex::Prompt);
    assert_eq!(only(|b| b.write_warning = true), CreateIndex::WriteWarning);
    assert_eq!(CreateIndex::read_only(true), CreateIndex::ReadOnly);
}

#[test]
fn read_only_wins_then_the_write_in_flight_then_the_warning() {
    let all = CreateBlockers {
        read_only: true,
        index_write: true,
        prompt: true,
        write_warning: true,
    };
    assert_eq!(CreateIndex::of(all), CreateIndex::ReadOnly);
    let no_read_only = CreateBlockers {
        read_only: false,
        ..all
    };
    assert_eq!(CreateIndex::of(no_read_only), CreateIndex::IndexWrite);
    let no_write = CreateBlockers {
        index_write: false,
        ..no_read_only
    };
    assert_eq!(CreateIndex::of(no_write), CreateIndex::WriteWarning);
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
