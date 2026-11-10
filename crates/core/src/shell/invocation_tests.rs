use super::*;
use crate::connection::AuthMechanism;

fn cfg(uri: &str) -> ConnectionConfig {
    ConnectionConfig::new("a", uri)
}

fn scram(uri: &str) -> ConnectionConfig {
    let mut c = cfg(uri);
    c.auth = AuthMechanism::ScramSha256;
    c.username = Some("bob".into());
    c
}

fn preamble_text(inv: &Invocation) -> String {
    String::from_utf8(inv.preamble.clone()).unwrap()
}

fn build(c: &ConnectionConfig, pw: Option<&str>, tunnel: Option<SocketAddr>) -> Invocation {
    build_invocation(c, pw, tunnel, "db.a.find()").unwrap()
}

fn addr() -> SocketAddr {
    "127.0.0.1:50000".parse().unwrap()
}

#[test]
fn password_is_only_in_the_preamble() {
    let pw = "s3cr3t-Pa55";
    let inv = build_invocation(
        &scram("mongodb://h:27017/admin"),
        Some(pw),
        None,
        "db.a.find()",
    )
    .unwrap();
    assert!(preamble_text(&inv).contains(pw));
    for arg in &inv.args {
        assert!(!arg.to_string_lossy().contains(pw));
    }
    assert!(!String::from_utf8_lossy(&inv.script).contains(pw));
}

#[test]
fn special_characters_are_percent_encoded() {
    let table = [
        ("@", "%40"),
        (":", "%3A"),
        ("/", "%2F"),
        ("%", "%25"),
        ("?", "%3F"),
        ("#", "%23"),
        ("\"", "%22"),
        ("'", "%27"),
        ("\\", "%5C"),
        (" ", "%20"),
    ];
    for (raw, encoded) in table {
        assert_eq!(percent_encode_component(raw), encoded, "{raw:?}");
        let pw = format!("a{raw}b");
        let inv = build(&scram("mongodb://h/admin"), Some(&pw), None);
        let expected = format!("mongodb://bob:a{encoded}b@h/admin");
        assert!(preamble_text(&inv).contains(&expected), "{raw:?}");
    }
    assert_eq!(percent_encode_component("Az09-._~"), "Az09-._~");
    assert_eq!(percent_encode_component("é"), "%C3%A9");
}

#[test]
fn newline_in_password_cannot_end_the_connect_line() {
    let inv = build(&scram("mongodb://h/admin"), Some("a\nb\r\nc"), None);
    let text = preamble_text(&inv);
    assert!(text.ends_with('\n'));
    assert_eq!(text.matches('\n').count(), 1);
    assert!(!text.contains('\r'));
    assert!(text.contains("a%0Ab%0D%0Ac"));
}

#[test]
fn uri_options_with_quotes_and_backslashes_stay_inside_the_string_literal() {
    let inv = build(&cfg("mongodb://h/admin?appName=a\"b\\c"), None, None);
    let text = preamble_text(&inv);
    assert_eq!(text.matches('\n').count(), 1);
    assert!(text.contains("appName=a\\\"b\\\\c"));
}

#[test]
fn preamble_has_the_sentinels() {
    let text = preamble_text(&build(&cfg("mongodb://h/admin"), None, None));
    assert!(text.contains("try { db = connect(\"mongodb://h/admin\"); "));
    assert!(text.contains("print(\"__CB_READY__\")"));
    assert!(text.contains("print(\"__CB_FAILED__ \" + e.message)"));
}

#[test]
fn preamble_fixes_the_prompt_before_connecting() {
    let text = preamble_text(&build(&cfg("mongodb://h/admin"), None, None));
    assert!(
        text.starts_with("prompt = () => \"\\u0001CBP> \"; "),
        "{text}"
    );
    let prompt_at = text.find("prompt = ").unwrap();
    assert!(
        prompt_at < text.find("try { db = connect(").unwrap(),
        "{text}"
    );
    // The JS escape and the token the classifier strips are the same text.
    assert_eq!(PROMPT, "\u{1}CBP> ");
    assert_eq!(text.matches('\n').count(), 1);
}

#[test]
fn preamble_makes_password_prompt_throw_before_the_script_runs() {
    // Unblocked, `passwordPrompt()` reads the rest of the piped script as the "password".
    let text = preamble_text(&build(&cfg("mongodb://h/admin"), Some("pw"), None));
    let blocked = "passwordPrompt = () => { throw new Error(\"passwordPrompt() is not available in the shell tab\") }; ";
    let at = text.find(blocked).expect("the passwordPrompt override");
    assert!(at < text.find("try { db = connect(").unwrap(), "{text}");
    assert_eq!(text.matches('\n').count(), 1, "still one line");
}

#[test]
fn tunnel_replaces_hosts_with_local_addr() {
    let inv = build(
        &scram("mongodb://a.internal:27017,b.internal:27017/admin?replicaSet=rs"),
        Some("pw"),
        Some(addr()),
    );
    let text = preamble_text(&inv);
    assert!(text.contains("mongodb://bob:pw@127.0.0.1:50000/admin?"));
    assert!(text.contains("replicaSet=rs"));
    assert!(text.contains("directConnection=true"));
    assert!(!text.contains("internal"));
}

#[test]
fn srv_with_tunnel_is_rejected() {
    let err = build_invocation(
        &cfg("mongodb+srv://cluster0.example.com/db"),
        None,
        Some(addr()),
        "1",
    )
    .err();
    assert_eq!(err, Some(ShellError::SrvWithTunnel));
}

#[test]
fn verified_tls_with_tunnel_is_rejected() {
    let mut c = cfg("mongodb://db.internal:27017");
    c.tls.enabled = true;
    assert_eq!(
        build_invocation(&c, None, Some(addr()), "1").err(),
        Some(ShellError::UnverifiableTls)
    );
    assert!(build_invocation(&c, None, None, "1").is_ok());
    c.tls.allow_invalid_certs = true;
    assert!(build_invocation(&c, None, Some(addr()), "1").is_ok());
}

#[test]
fn tls_ca_and_client_cert_become_uri_options() {
    let mut c = cfg("mongodb://h/admin?retryWrites=true");
    c.tls.enabled = true;
    c.tls.ca_file = Some("/tmp/my ca.pem".into());
    c.tls.client_cert_key_file = Some("/tmp/c&d.pem".into());
    c.tls.allow_invalid_certs = true;
    let text = preamble_text(&build(&c, None, None));
    assert!(text.contains("retryWrites=true"));
    assert!(text.contains("tls=true"));
    assert!(text.contains("tlsCAFile=%2Ftmp%2Fmy%20ca.pem"));
    assert!(text.contains("tlsCertificateKeyFile=%2Ftmp%2Fc%26d.pem"));
    assert!(text.contains("tlsAllowInvalidCertificates=true"));
}

#[test]
fn x509_and_gssapi_use_external_auth_source() {
    let mut x = cfg("mongodb://h");
    x.auth = AuthMechanism::X509;
    x.tls.enabled = true;
    x.tls.client_cert_key_file = Some("/tmp/c.pem".into());
    let text = preamble_text(&build(&x, None, None));
    assert!(text.contains("authSource=%24external"));
    assert!(text.contains("authMechanism=MONGODB-X509"));

    let mut g = cfg("mongodb://h/db");
    g.auth = AuthMechanism::Gssapi;
    g.username = Some("user@REALM".into());
    let text = preamble_text(&build(&g, None, None));
    assert!(text.contains("mongodb://user%40REALM@h/db?"));
    assert!(text.contains("authSource=%24external"));
    assert!(text.contains("authMechanism=GSSAPI"));

    let mut s = scram("mongodb://h/db");
    s.auth_source = Some("admin".into());
    let text = preamble_text(&build(&s, Some("pw"), None));
    assert!(text.contains("authSource=admin"));
    assert!(text.contains("authMechanism=SCRAM-SHA-256"));
}

#[test]
fn secrets_in_the_stored_uri_are_rejected() {
    let err = build_invocation(
        &cfg("mongodb://h/db?tlsCertificateKeyFilePassword=hunter2"),
        None,
        None,
        "1",
    )
    .err();
    assert_eq!(err, Some(ShellError::Uri(UriProblem::SecretInUri)));
    let err = build_invocation(&cfg("mongodb://bob:pw@h/db"), None, None, "1").err();
    assert_eq!(err, Some(ShellError::Uri(UriProblem::SecretInUri)));
    let err = build_invocation(&cfg("http://h"), None, None, "1").err();
    assert_eq!(err, Some(ShellError::Uri(UriProblem::MissingScheme)));
}

#[test]
fn empty_or_whitespace_script_is_rejected() {
    for script in ["", "   ", "\n\t \r\n"] {
        let err = build_invocation(&cfg("mongodb://h"), None, None, script).err();
        assert_eq!(err, Some(ShellError::EmptyScript), "{script:?}");
    }
}

#[test]
fn the_script_is_sent_as_one_block_with_the_brace_on_its_first_line() {
    let script = "  db.a.find({x: 'é'})\nprint(1)  ";
    let inv = build_invocation(&cfg("mongodb://h"), None, None, script).unwrap();
    assert_eq!(
        inv.script,
        "{  db.a.find({x: 'é'})\nprint(1)  \n}\n".as_bytes()
    );
}

#[test]
fn every_line_break_mongosh_splits_on_becomes_a_newline() {
    // mongosh's line reader also ends a line at `\r` and at U+2028/U+2029; as `\n` the lines the
    // runner writes are exactly the lines mongosh reads.
    let block = script_block("a\r\nb\rc\u{2028}d\u{2029}e");
    assert_eq!(block, "{a\nb\nc\nd\ne\n}\n".as_bytes());
}

#[test]
fn args_are_exactly_nodb_and_quiet() {
    let inv = build(&cfg("mongodb://h"), None, None);
    assert_eq!(
        inv.args,
        vec![OsString::from("--nodb"), OsString::from("--quiet")]
    );
}

#[test]
fn name_and_folder_do_not_block_the_shell() {
    let mut c = cfg("mongodb://h/admin");
    c.name = String::new();
    c.folder = Some(" odd\u{7} ".into());
    assert!(build_invocation(&c, None, None, "1").is_ok());
}

#[test]
fn bad_uri_and_x509_without_cert_keep_their_problems() {
    let err = build_invocation(&cfg("mongodb://"), None, None, "1").err();
    assert_eq!(err, Some(ShellError::Uri(UriProblem::MissingHost)));
    let mut x = cfg("mongodb://h");
    x.auth = AuthMechanism::X509;
    let err = build_invocation(&x, None, None, "1").err();
    assert_eq!(err, Some(ShellError::Uri(UriProblem::X509Needs)));
}

#[test]
fn debug_output_never_shows_the_password_or_the_uri() {
    let pw = "s3cr3t-Pa55:@/";
    let inv = build(&scram("mongodb://db.example:27017/admin"), Some(pw), None);
    for text in [format!("{inv:?}"), format!("{inv:#?}")] {
        assert!(!text.contains("s3cr3t"));
        assert!(!text.contains(&percent_encode_component(pw)));
        assert!(!text.contains("db.example"));
        assert!(!text.contains("mongodb://"));
        assert!(text.contains("<redacted>"));
        assert!(text.contains("--nodb"));
    }
}

#[test]
fn line_separators_and_template_openers_keep_the_preamble_one_line() {
    let c = cfg("mongodb://h/admin?appName=x\u{2028}y\u{2029}${z}");
    let mut s = c.clone();
    s.auth = AuthMechanism::ScramSha256;
    s.username = Some("bob".into());
    for (cfg, pw) in [(c, None), (s, Some("a\u{2028}b\u{2029}c${d}`"))] {
        let text = preamble_text(&build(&cfg, pw, None));
        assert_eq!(text.matches('\n').count(), 1);
        assert!(!text.contains('\u{2028}') && !text.contains('\u{2029}'));
    }
}

#[test]
fn ipv6_tunnel_address_is_bracketed() {
    let v6: SocketAddr = "[::1]:50000".parse().unwrap();
    let text = preamble_text(&build(&cfg("mongodb://h/admin"), None, Some(v6)));
    assert!(text.contains("mongodb://[::1]:50000/admin?"));
}
