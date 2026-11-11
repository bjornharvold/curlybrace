use super::*;

fn run(secrets: &[&str], chunks: &[&str]) -> String {
    let mut scrubber = Scrubber::new(secrets);
    let mut out: String = chunks.iter().map(|c| scrubber.push(c)).collect();
    out.push_str(&scrubber.finish());
    out
}

#[test]
fn replaces_secret_in_one_chunk() {
    assert_eq!(run(&["pass"], &["a pass b"]), format!("a {MASK} b"));
}

#[test]
fn replaces_secret_split_across_two_chunks() {
    let mut scrubber = Scrubber::new(&["pass"]);
    let first = scrubber.push("pa");
    let second = scrubber.push("ss");
    assert_eq!(format!("{first}{second}{}", scrubber.finish()), MASK);
    assert!(!first.contains("pa"));
}

#[test]
fn holds_back_only_a_possible_prefix() {
    let mut scrubber = Scrubber::new(&["pass"]);
    assert_eq!(scrubber.push("hello x"), "hello x");
    assert_eq!(scrubber.push("hello p"), "hello ");
    assert_eq!(scrubber.finish(), "p");
}

#[test]
fn finish_flushes_held_tail() {
    let mut scrubber = Scrubber::new(&["pass"]);
    assert_eq!(scrubber.push("ends with pas"), "ends with ");
    assert_eq!(scrubber.finish(), "pas");
    assert_eq!(scrubber.finish(), "");
}

#[test]
fn no_secret_passes_through_unchanged() {
    assert_eq!(run(&["pass"], &["nothing ", "to see"]), "nothing to see");
    assert_eq!(run(&[], &["héllo ••• wörld"]), "héllo ••• wörld");
}

#[test]
fn empty_secret_is_ignored() {
    assert_eq!(run(&[""], &["abc"]), "abc");
    assert_eq!(run(&["", "pass"], &["a pass"]), format!("a {MASK}"));
}

#[test]
fn secret_that_is_a_substring_of_masked_output_is_not_rescrubbed() {
    assert_eq!(run(&["•"], &["a•b"]), format!("a{MASK}b"));
    assert_eq!(run(&["•", "x"], &["•x"]), format!("{MASK}{MASK}"));
}

#[test]
fn masks_every_listed_secret_including_the_percent_encoded_form() {
    let out = run(&["p@ss", "p%40ss"], &["raw p@ss and uri p%40", "ss end"]);
    assert_eq!(out, format!("raw {MASK} and uri {MASK} end"));
}

#[test]
fn longer_secret_wins_when_one_contains_the_other() {
    assert_eq!(
        run(&["pass", "password"], &["a password b"]),
        format!("a {MASK} b")
    );
    assert_eq!(
        run(&["pass", "password"], &["a pass", "word b"]),
        format!("a {MASK} b")
    );
    assert_eq!(
        run(&["pass", "password"], &["a pass", "!"]),
        format!("a {MASK}!")
    );
}

#[test]
fn non_ascii_secret_and_text_stay_on_char_boundaries() {
    assert_eq!(
        run(&["pässwörd"], &["x pä", "sswörd y é"]),
        format!("x {MASK} y é")
    );
    assert_eq!(run(&["ö"], &["aöb"]), format!("a{MASK}b"));
}

#[test]
fn finish_masks_a_complete_secret_that_was_held_as_a_prefix_of_a_longer_one() {
    let mut scrubber = Scrubber::new(&["pass", "password"]);
    let shown = scrubber.push("a pass");
    assert_eq!(format!("{shown}{}", scrubber.finish()), format!("a {MASK}"));

    let mut scrubber = Scrubber::new(&["a%", "a%25"]);
    let shown = scrubber.push("x a%");
    assert_eq!(format!("{shown}{}", scrubber.finish()), format!("x {MASK}"));
}

#[test]
fn overlapping_occurrences_are_masked_consistently_whole_and_chunked() {
    let whole = run(&["aa"], &["aaa"]);
    let chars: Vec<String> = "aaa".chars().map(String::from).collect();
    let chunks: Vec<&str> = chars.iter().map(String::as_str).collect();
    assert_eq!(whole, format!("{MASK}a"));
    assert_eq!(run(&["aa"], &chunks), whole);
}

#[test]
fn carry_stays_bounded_over_a_long_partial_match_stream() {
    let mut scrubber = Scrubber::new(&["abcd"]);
    for _ in 0..200 {
        for ch in ["a", "b", "c"] {
            scrubber.push(ch);
            assert!(scrubber.carry.len() < "abcd".len());
        }
    }
}
