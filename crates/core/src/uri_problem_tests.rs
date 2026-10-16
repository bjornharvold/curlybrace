//! Which `UriProblem` each class of bad input gets, and that no error ever carries the input.
use crate::connection::ConnectionConfig;
use crate::error::CoreError;
use crate::uri::{check_hosts, parse_uri};
use crate::uri_problem::UriProblem::{self, *};

const TOKEN: &str = "S3CR3T";

fn parse_problem(input: &str) -> Option<UriProblem> {
    match parse_uri(input) {
        Err(CoreError::InvalidUri(p)) => Some(p),
        Err(other) => panic!("{input}: unexpected {other:?}"),
        Ok(_) => None,
    }
}

fn check_problem(input: &str) -> Option<UriProblem> {
    match check_hosts(input) {
        Err(CoreError::InvalidUri(p)) => Some(p),
        Err(other) => panic!("{input}: unexpected {other:?}"),
        Ok(()) => None,
    }
}

/// (input, problem) pairs both entry points must agree on.
fn shared_cases() -> Vec<(String, UriProblem)> {
    let t = TOKEN;
    let cases: Vec<(String, UriProblem)> = vec![
        ("h:27017".into(), MissingScheme),
        ("".into(), MissingScheme),
        ("http://h".into(), MissingScheme),
        ("mongodb:/h".into(), MissingScheme),
        ("mongodb://".into(), MissingHost),
        ("mongodb://u@".into(), MissingHost),
        ("mongodb://u@/db".into(), MissingHost),
        ("mongodb+srv://".into(), MissingHost),
        ("mongodb://h,".into(), BadHost),
        ("mongodb://h,,h2".into(), BadHost),
        ("mongodb://h h2".into(), BadHost),
        ("mongodb://[::1".into(), BadHost),
        ("mongodb://[zz]:1".into(), BadHost),
        ("mongodb://[::1]x".into(), BadHost),
        ("mongodb://%2Ftmp%2Fx".into(), BadHost),
        ("mongodb://h$x".into(), BadHost),
        ("mongodb://h:".into(), BadPort),
        ("mongodb://h:0".into(), BadPort),
        ("mongodb://h:65536".into(), BadPort),
        ("mongodb://h:12a".into(), BadPort),
        (format!("mongodb://u:pw@h:{t}"), BadPort),
        ("mongodb+srv://a.example.com,b.example.com".into(), SrvRules),
        ("mongodb+srv://a.example.com:27017".into(), SrvRules),
        (format!("mongodb://a:b@c@h/db?x={t}"), StrayAtSign),
        (format!("mongodb://bob:p@{t}@h/db"), StrayAtSign),
        (format!("mongodb://admin:2024/{t}@db"), StrayAtSign),
        (format!("mongodb://u:12345?{t}@h"), StrayAtSign),
        (format!("mongodb://u:pa/{t}@h"), StrayAtSign),
        (format!("mongodb://h/db@{t}"), StrayAtSign),
        (format!("mongodb://h/?a=b&{t}@h"), StrayAtSign),
        (format!("mongodb://h/?x@{t}=1"), StrayAtSign),
        ("mongodb://h#frag".into(), Other),
    ];
    cases
}

/// Problems only `parse_uri` reads (credentials and options are not part of `check_hosts`).
fn parse_only_cases() -> Vec<(String, UriProblem)> {
    let t = TOKEN;
    vec![
        (
            format!("mongodb://u:p@h/?tlsCertificateKeyFilePassword={t}"),
            SecretInUri,
        ),
        (
            format!("mongodb://u:p@h/?sslClientCertificateKeyPassword={t}"),
            SecretInUri,
        ),
        (format!("mongodb://u:p@h/?ProxyPassword={t}"), SecretInUri),
        (format!("mongodb://u:%zz{t}@h"), BadPercentEncoding),
        (format!("mongodb://u%4@h/?x={t}"), BadPercentEncoding),
        (format!("mongodb://u:%C3{t}@h"), BadPercentEncoding),
        (
            format!("mongodb://h/?authSource=%zz{t}"),
            BadPercentEncoding,
        ),
        (format!("mongodb://h/db?a&x={t}"), BadOption),
        (format!("mongodb://h/db?x={t}&b"), BadOption),
        (format!("mongodb://u:p@h/?authMechanism=NOPE{t}"), BadOption),
        (format!("mongodb://h/?tls=maybe{t}"), BadOption),
        (format!("mongodb://:{t}@h"), Other),
    ]
}

#[test]
fn parse_uri_returns_the_right_problem_for_each_input_class() {
    for (input, expected) in shared_cases().into_iter().chain(parse_only_cases()) {
        assert_eq!(parse_problem(&input), Some(expected), "{input}");
    }
}

#[test]
fn check_hosts_returns_the_same_problem_for_the_shared_classes() {
    for (input, expected) in shared_cases() {
        assert_eq!(check_problem(&input), Some(expected), "{input}");
    }
}

#[test]
fn a_stray_at_sign_in_a_query_value_is_still_accepted() {
    assert!(parse_uri("mongodb://h/?appName=bob@laptop").is_ok());
    assert!(check_hosts("mongodb://h/?appName=bob@laptop").is_ok());
}

#[test]
fn no_error_text_or_debug_ever_contains_the_input() {
    let all: Vec<String> = shared_cases()
        .into_iter()
        .chain(parse_only_cases())
        .map(|(input, _)| input)
        .collect();
    for input in &all {
        for err in [parse_uri(input).err(), check_hosts(input).err()]
            .into_iter()
            .flatten()
        {
            let shown = format!("{err} | {err:?} | {}", err.i18n_key());
            assert!(!shown.contains(TOKEN), "{input}: {shown}");
        }
    }
}

#[test]
fn validate_uri_problems_are_typed_and_secret_free() {
    let cfg = |uri: &str| ConnectionConfig::new("a", uri).validate();
    assert_eq!(cfg("h"), Err(CoreError::InvalidUri(MissingScheme)));
    assert_eq!(cfg("mongodb://"), Err(CoreError::InvalidUri(MissingHost)));
    let err = cfg(&format!("mongodb://bob:{TOKEN}@h/db")).unwrap_err();
    assert_eq!(err, CoreError::InvalidUri(SecretInUri));
    assert!(!format!("{err} {err:?}").contains(TOKEN));
}

#[test]
fn the_variants_map_to_distinct_keys_through_the_core_error() {
    let mut keys: Vec<_> = UriProblem::ALL
        .iter()
        .map(|p| CoreError::InvalidUri(*p).i18n_key())
        .collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), UriProblem::ALL.len());
}
