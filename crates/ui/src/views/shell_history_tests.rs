use super::*;

const ALL: [ShellRunStatus; 6] = [
    ShellRunStatus::Succeeded,
    ShellRunStatus::ScriptErrors,
    ShellRunStatus::Incomplete,
    ShellRunStatus::Cancelled,
    ShellRunStatus::CapReached,
    ShellRunStatus::Failed,
];

#[test]
fn rows_have_ids_of_their_own_per_session_and_entry() {
    let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
    assert_eq!(row_id(a, 7), row_id(a, 7));
    assert_ne!(row_id(a, 7), row_id(b, 7));
    assert_ne!(row_id(a, 7), row_id(a, 8));
}

#[test]
fn every_status_has_its_own_words() {
    let i18n = I18n::english();
    let words: std::collections::BTreeSet<_> = ALL.iter().map(|s| i18n.t(status_key(*s))).collect();
    assert_eq!(words.len(), ALL.len());
    assert!(words.iter().all(|w| !w.starts_with("shell.")), "{words:?}");
}

#[test]
fn a_failure_reads_bad_and_a_success_good() {
    assert_eq!(status_tone(ShellRunStatus::Succeeded), Tone::Good);
    assert_eq!(status_tone(ShellRunStatus::Failed), Tone::Bad);
    assert_eq!(status_tone(ShellRunStatus::ScriptErrors), Tone::Bad);
    assert_eq!(status_tone(ShellRunStatus::Cancelled), Tone::Neutral);
}

#[test]
fn the_time_is_said_in_the_coarsest_unit_and_never_in_the_future() {
    let i18n = I18n::english();
    let now = 1_000_000;
    let minutes = i18n.t_with("indexes.ago_minutes", &[("n", "5")]);
    assert_eq!(ago(now - 300, now, &i18n), minutes);
    let just_now = i18n.t_with("indexes.ago_now", &[("n", "0")]);
    assert_eq!(ago(now + 3_600, now, &i18n), just_now);
    // Extreme stored times do not overflow.
    assert!(!ago(i64::MIN, i64::MAX, &i18n).is_empty());
}
