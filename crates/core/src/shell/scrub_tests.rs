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

// ---- escaped forms of the password ----

/// A scrubber over every form of `password`, as the runtime builds it.
fn run_forms(password: &str, chunks: &[&str]) -> String {
    let forms = secret_forms(password);
    let refs: Vec<&str> = forms.iter().map(String::as_str).collect();
    run(&refs, chunks)
}

#[test]
fn a_plain_password_still_yields_the_raw_and_percent_encoded_forms() {
    assert_eq!(secret_forms("hunter2"), vec!["hunter2".to_owned()]);
    assert_eq!(
        secret_forms("p@ss:w/rd"),
        vec!["p@ss:w/rd".to_owned(), "p%40ss%3Aw%2Frd".to_owned()]
    );
}

#[test]
fn an_empty_password_yields_no_forms() {
    assert!(secret_forms("").is_empty());
}

#[test]
fn a_password_with_a_backslash_is_masked_in_its_doubled_form() {
    let forms = secret_forms(r"pa\ss");
    assert!(forms.contains(&r"pa\ss".to_owned()));
    assert!(forms.contains(&r"pa\\ss".to_owned()), "{forms:?}");
    assert_eq!(
        run_forms(r"pa\ss", &[r"echo 'pa\\ss' and raw pa\ss"]),
        format!("echo '{MASK}' and raw {MASK}")
    );
}

#[test]
fn a_password_with_a_quote_is_masked_in_its_json_escaped_form() {
    let forms = secret_forms(r#"pa"ss"#);
    assert!(forms.contains(&r#"pa\"ss"#.to_owned()), "{forms:?}");
    assert_eq!(
        run_forms(r#"pa"ss"#, &[r#"{"u":"pa\"ss"}"#]),
        format!(r#"{{"u":"{MASK}"}}"#)
    );
}

#[test]
fn a_password_with_a_newline_or_unicode_is_masked_in_json_form() {
    let password = "a\nb\té";
    let forms = secret_forms(password);
    // serde_json keeps é literal and escapes the control characters.
    assert!(forms.contains(&r"a\nb\té".to_owned()), "{forms:?}");
    assert_eq!(
        run_forms(password, &[r#"{"p":"a\nb\té"}"#]),
        format!(r#"{{"p":"{MASK}"}}"#)
    );
}

#[test]
fn a_password_with_a_control_character_is_masked_in_json_and_inspect_form() {
    assert_eq!(
        run_forms("a\u{1}b", &[r#"j "a\u0001b" i 'a\x01b'"#]),
        format!(r#"j "{MASK}" i '{MASK}'"#)
    );
}

#[test]
fn a_password_with_a_single_quote_is_masked_in_inspect_form() {
    let forms = secret_forms("pa'ss");
    // Single-quote style escapes the quote; the double-quote style does not.
    assert!(forms.contains(&r"pa\'ss".to_owned()), "{forms:?}");
    assert!(forms.contains(&"pa'ss".to_owned()), "{forms:?}");
    assert_eq!(
        run_forms("pa'ss", &[r"'pa\'ss' then "]),
        format!("'{MASK}' then ")
    );
    assert_eq!(
        run_forms("pa'ss", &[r#"then "pa'ss""#]),
        format!(r#"then "{MASK}""#)
    );
}

#[test]
fn a_password_with_a_quote_and_a_backslash_is_masked_in_every_inspect_style() {
    for shown in [r"a\\b\'c", r"a\\b'c"] {
        assert_eq!(run_forms(r"a\b'c", &[shown]), MASK, "{shown}");
    }
}

#[test]
fn forms_equal_to_the_raw_password_are_not_duplicated() {
    for password in ["plain", "p@ss", r"a\b", "a'b", "é"] {
        let forms = secret_forms(password);
        let mut unique = forms.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(forms.len(), unique.len(), "{forms:?}");
        assert_eq!(forms[0], password);
        assert!(forms.iter().all(|f| !f.is_empty()));
    }
    assert_eq!(secret_forms("é"), vec!["é".to_owned(), "%C3%A9".to_owned()]);
}

#[test]
fn the_escaped_form_is_masked_across_a_chunk_boundary() {
    assert_eq!(
        run_forms(r"pa\ss", &[r"x 'pa\", r"\ss' y"]),
        format!("x '{MASK}' y")
    );
    assert_eq!(
        run_forms(r#"pa"ss"#, &[r#"x "pa\"#, r#""ss" y"#]),
        format!(r#"x "{MASK}" y"#)
    );
}

#[test]
fn mask_covers_the_escaped_forms_too() {
    let forms = secret_forms(r"pa\ss");
    let refs: Vec<&str> = forms.iter().map(String::as_str).collect();
    let scrubber = Scrubber::new(&refs);
    assert_eq!(
        scrubber.mask(r"bad pa\\ss here"),
        format!("bad {MASK} here")
    );
}
