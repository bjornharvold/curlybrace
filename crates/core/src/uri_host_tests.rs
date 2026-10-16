//! Unicode and percent-encoded hosts: `parse_uri` (From URI) converts them to the ASCII form the
//! driver needs, `check_hosts` (save, import) never rewrites and accepts only that ASCII form.
use crate::error::CoreError;
use crate::uri::{check_hosts, parse_uri};
use crate::uri_problem::UriProblem::{self, *};

fn parsed_uri(input: &str) -> String {
    parse_uri(input)
        .unwrap_or_else(|e| panic!("{input}: {e}"))
        .uri
}

fn parse_problem(input: &str) -> UriProblem {
    match parse_uri(input) {
        Err(CoreError::InvalidUri(p)) => p,
        other => panic!("{input}: expected an InvalidUri, got {other:?}"),
    }
}

fn check_problem(input: &str) -> UriProblem {
    match check_hosts(input) {
        Err(CoreError::InvalidUri(p)) => p,
        other => panic!("{input}: expected an InvalidUri, got {other:?}"),
    }
}

#[test]
fn a_unicode_host_becomes_punycode() {
    assert_eq!(
        parsed_uri("mongodb://bücher.example/"),
        "mongodb://xn--bcher-kva.example/"
    );
    assert_eq!(
        parsed_uri("mongodb://пример.рф/db"),
        "mongodb://xn--e1afmkfd.xn--p1ai/db"
    );
    assert_eq!(
        parsed_uri("mongodb+srv://bücher.example/db"),
        "mongodb+srv://xn--bcher-kva.example/db"
    );
}

#[test]
fn the_port_is_kept_and_ascii_ipv6_and_sockets_are_untouched() {
    assert_eq!(
        parsed_uri("mongodb://bücher.example:27018"),
        "mongodb://xn--bcher-kva.example:27018"
    );
    assert_eq!(
        parsed_uri("mongodb://a.example,bücher.example:27018,[::1]:27019,%2Ftmp%2Fm-1.sock"),
        "mongodb://a.example,xn--bcher-kva.example:27018,[::1]:27019,%2Ftmp%2Fm-1.sock"
    );
}

#[test]
fn an_uppercase_unicode_host_is_lowercased_by_the_conversion() {
    assert_eq!(
        parsed_uri("mongodb://BÜCHER.Example"),
        "mongodb://xn--bcher-kva.example"
    );
}

#[test]
fn credentials_and_options_survive_a_host_conversion() {
    let p = parse_uri("mongodb://bob:pw@bücher.example/db?replicaSet=rs0&tls=true").unwrap();
    assert_eq!(p.uri, "mongodb://xn--bcher-kva.example/db?replicaSet=rs0");
    assert_eq!(p.username.as_deref(), Some("bob"));
    assert_eq!(p.tls, Some(true));
}

#[test]
fn an_invalid_unicode_host_is_a_bad_host() {
    let long_label = "ü".repeat(64);
    for bad in [
        "mongodb://\u{0301}.example".to_owned(),
        "mongodb://\u{0301}".to_owned(),
        format!("mongodb://{long_label}.example"),
        format!("mongodb://a{}ü.example", "a".repeat(70)),
        "mongodb://ü..example".to_owned(),
        "mongodb://bü cher.example".to_owned(),
        "mongodb://bü\u{200B}cher.example".to_owned(),
        "mongodb://bü／cher.example".to_owned(),
        "mongodb://bü＠cher.example".to_owned(),
        "mongodb://bü\u{0}cher.example".to_owned(),
        "mongodb://ü,".to_owned(),
    ] {
        assert_eq!(parse_problem(&bad), BadHost, "{bad:?}");
    }
}

#[test]
fn a_percent_encoded_host_is_decoded_then_validated() {
    assert_eq!(parsed_uri("mongodb://h%2Eexample"), "mongodb://h.example");
    assert_eq!(
        parsed_uri("mongodb://h%2Eexample:27017,H2%2dx"),
        "mongodb://h.example:27017,H2-x"
    );
    assert_eq!(
        parsed_uri("mongodb://b%C3%BCcher.example"),
        "mongodb://xn--bcher-kva.example"
    );
    assert_eq!(
        parsed_uri("mongodb://b%c3%bcCHER.example/db"),
        "mongodb://xn--bcher-kva.example/db"
    );
}

#[test]
fn a_decoded_host_with_a_forbidden_character_is_rejected() {
    for bad in [
        "mongodb://h%2F",
        "mongodb://h%2Fx",
        "mongodb://h%2fx.example",
        "mongodb://h%40x",
        "mongodb://h%3Ax",
        "mongodb://h%20x",
        "mongodb://h%09x",
        "mongodb://h%00x",
        "mongodb://h%0Ax",
        "mongodb://h%25x",
        "mongodb://h%2Cx",
        "mongodb://h%5Bx",
        "mongodb://h%3Fx",
        "mongodb://h%23x",
        "mongodb://h%C2%A0x",
    ] {
        assert_eq!(parse_problem(bad), BadHost, "{bad}");
    }
}

#[test]
fn a_broken_percent_encoding_in_a_host_is_reported_as_such() {
    for bad in [
        "mongodb://h%zz",
        "mongodb://h%",
        "mongodb://h%2",
        "mongodb://h%FF.example",
    ] {
        assert_eq!(parse_problem(bad), BadPercentEncoding, "{bad}");
    }
}

#[test]
fn a_percent_encoded_port_is_a_bad_port_not_a_host() {
    assert_eq!(parse_problem("mongodb://h:%32%37"), BadPort);
}

#[test]
fn srv_rules_still_apply_to_unicode_hosts() {
    assert_eq!(
        parse_problem("mongodb+srv://bücher.example:27017"),
        SrvRules
    );
    assert_eq!(
        parse_problem("mongodb+srv://bücher.example,a.example"),
        SrvRules
    );
}

#[test]
fn check_hosts_never_rewrites_and_flags_a_unicode_host() {
    for bad in [
        "mongodb://bücher.example/",
        "mongodb://a.example,пример.рф:27017",
        "mongodb+srv://bücher.example",
        "mongodb://u@bücher.example/db",
    ] {
        assert_eq!(check_problem(bad), NonAsciiHost, "{bad}");
    }
    assert!(check_hosts("mongodb://xn--bcher-kva.example/").is_ok());
}

#[test]
fn check_hosts_does_not_decode_a_percent_encoded_host() {
    for bad in ["mongodb://h%2Eexample", "mongodb://b%C3%BCcher.example"] {
        assert_eq!(check_problem(bad), BadHost, "{bad}");
    }
    assert!(check_hosts("mongodb://%2Ftmp%2Fmongodb-27017.sock").is_ok());
}

#[test]
fn what_parse_uri_produces_always_passes_check_hosts() {
    for input in [
        "mongodb://bücher.example/",
        "mongodb://h%2Eexample:1,b%C3%BCcher.example",
        "mongodb+srv://пример.рф/db",
    ] {
        let uri = parsed_uri(input);
        assert!(check_hosts(&uri).is_ok(), "{input} -> {uri}");
    }
}

#[test]
fn host_errors_never_carry_the_host() {
    for input in [
        "mongodb://S3CR3Tü.\u{0301}",
        "mongodb://S3CR3T%2F",
        "mongodb://S3CR3T%zz",
        "mongodb://S3CR3Tü.example",
        "mongodb://u:S3CR3T@S3CR3T%40x",
    ] {
        for err in [parse_uri(input).err(), check_hosts(input).err()]
            .into_iter()
            .flatten()
        {
            let shown = format!("{err} {err:?} {}", err.i18n_key());
            assert!(!shown.contains("S3CR3T"), "{input}: {shown}");
        }
    }
}

#[test]
fn huge_inputs_do_not_blow_up() {
    let wide = "ü".repeat(100_000);
    assert_eq!(parse_problem(&format!("mongodb://{wide}.example")), BadHost);
    assert_eq!(
        check_problem(&format!("mongodb://{wide}.example")),
        NonAsciiHost
    );
    let encoded = "%C3%BC".repeat(50_000);
    assert_eq!(parse_problem(&format!("mongodb://{encoded}")), BadHost);
    let many = vec!["bücher.example:27017"; 5_000].join(",");
    let uri = parsed_uri(&format!("mongodb://{many}/db"));
    assert_eq!(uri.matches("xn--bcher-kva.example:27017").count(), 5_000);
    let long_ascii = "a".repeat(200_000);
    assert!(parse_uri(&format!("mongodb://{long_ascii}")).is_ok());
}

#[test]
fn odd_input_never_panics() {
    for s in [
        "mongodb://\u{FFFD}",
        "mongodb://\u{10FFFF}",
        "mongodb://\u{1F600}.example",
        "mongodb://xn--",
        "mongodb://xn--a.example",
        "mongodb://xn--\u{FC}.example",
        "mongodb://ǅ.example",
        "mongodb://ß.example",
        "mongodb://%C3",
        "mongodb://[ü]:1",
    ] {
        let _ = parse_uri(s);
        let _ = check_hosts(s);
    }
}
