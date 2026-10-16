//! Zero-width joiners (U+200C, U+200D) are legitimate inside Persian and Indic IDN hosts. The
//! conversion's own joiner rules decide where they are valid; every other invisible character stays
//! refused and a joiner is never silently dropped.
use crate::error::CoreError;
use crate::uri::parse_uri;
use crate::uri_problem::UriProblem::{self, *};
use idna::uts46::{AsciiDenyList, Hyphens, Uts46};

const PERSIAN: &str = "می\u{200C}خواهم.ir";
const DEVANAGARI: &str = "क्\u{200D}ष.in";
const KANNADA: &str = "ಕ್\u{200C}ಷ.in";

fn host_of(input: &str) -> String {
    let uri =
        parse_uri(&format!("mongodb://{input}/db")).unwrap_or_else(|e| panic!("{input}: {e}"));
    uri.uri
        .strip_prefix("mongodb://")
        .and_then(|r| r.strip_suffix("/db"))
        .unwrap()
        .to_owned()
}

fn problem(input: &str) -> UriProblem {
    match parse_uri(&format!("mongodb://{input}/db")) {
        Err(CoreError::InvalidUri(p)) => p,
        other => panic!("{input:?}: expected an InvalidUri, got {other:?}"),
    }
}

fn decoded(ascii: &str) -> String {
    let (unicode, result) =
        Uts46::new().to_unicode(ascii.as_bytes(), AsciiDenyList::EMPTY, Hyphens::Allow);
    result.unwrap();
    unicode.into_owned()
}

#[test]
fn persian_and_indic_hosts_with_a_joiner_convert_and_keep_it() {
    let cases = [
        (PERSIAN, "xn--mgbn2ecje63gr19l.ir"),
        (DEVANAGARI, "xn--11b2ezcw70k.in"),
        (KANNADA, "xn--nsc2ezc870i.in"),
    ];
    for (typed, puny) in cases {
        let stored = host_of(typed);
        assert_eq!(stored, puny, "{typed:?}");
        let back = decoded(&stored);
        assert!(
            back.contains(['\u{200C}', '\u{200D}']),
            "joiner lost: {typed:?} -> {stored} -> {back:?}"
        );
        assert_eq!(back, typed);
    }
}

#[test]
fn a_joiner_in_an_invalid_context_is_a_bad_host() {
    for host in [
        "\u{200C}abc.example",
        "ab\u{200C}c.example",
        "bü\u{200D}cher.example",
        "\u{200D}.example",
        "\u{200C}",
        "a\u{200C}",
    ] {
        assert_eq!(problem(host), BadHost, "{host:?}");
    }
}

#[test]
fn the_other_invisible_characters_stay_refused() {
    for c in ['\u{200B}', '\u{200E}', '\u{200F}'] {
        for host in [format!("می{c}خواهم.ir"), format!("bü{c}cher.example")] {
            assert_eq!(problem(&host), BadHost, "{host:?}");
        }
    }
}

#[test]
fn the_idns_of_the_previous_round_still_convert() {
    for (typed, puny) in [
        ("bücher.example", "xn--bcher-kva.example"),
        ("пример.рф", "xn--e1afmkfd.xn--p1ai"),
        ("中文.com", "xn--fiq228c.com"),
    ] {
        assert_eq!(host_of(typed), puny);
    }
    for typed in ["مثال.إختبار", "ตัวอย่าง.ไทย", "예시.한국"] {
        let stored = host_of(typed);
        assert!(
            stored.is_ascii() && stored.contains("xn--"),
            "{typed}: {stored}"
        );
        assert_eq!(decoded(&stored), typed);
    }
}

#[test]
fn a_percent_encoded_joiner_converts_and_save_still_flags_the_host() {
    assert_eq!(
        host_of("%E0%B2%95%E0%B3%8D%E2%80%8C%E0%B2%B7.in"),
        "xn--nsc2ezc870i.in"
    );
    let uri = format!("mongodb://{PERSIAN}/db");
    assert_eq!(
        crate::uri::check_hosts(&uri),
        Err(CoreError::InvalidUri(NonAsciiHost))
    );
}
