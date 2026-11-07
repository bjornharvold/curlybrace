use super::*;

#[test]
fn mongosh_prints_a_bare_version() {
    assert_eq!(
        ToolVersion::parse("2.3.1\n"),
        Some(ToolVersion::new(2, 3, 1))
    );
}

#[test]
fn the_database_tools_print_a_labelled_first_line() {
    let out =
        "mongodump version: 100.10.0\ngit version: 1a2b3c\nGo version: go1.22.5\n   os: darwin\n";
    assert_eq!(ToolVersion::parse(out), Some(ToolVersion::new(100, 10, 0)));
}

#[test]
fn a_warning_before_the_version_is_skipped() {
    let out = "(node:123) Warning: something\n2.1.5\n";
    assert_eq!(ToolVersion::parse(out), Some(ToolVersion::new(2, 1, 5)));
}

#[test]
fn a_suffix_after_the_patch_number_is_ignored() {
    assert_eq!(
        ToolVersion::parse("mongosh 2.3.1-rc1"),
        Some(ToolVersion::new(2, 3, 1))
    );
}

#[test]
fn output_without_a_version_is_none() {
    assert_eq!(ToolVersion::parse(""), None);
    assert_eq!(ToolVersion::parse("command not found"), None);
    assert_eq!(ToolVersion::parse("1.2"), None);
}

#[test]
fn versions_order_by_major_then_minor_then_patch() {
    assert!(ToolVersion::new(2, 0, 0) > ToolVersion::new(1, 9, 9));
    assert!(ToolVersion::new(2, 3, 0) > ToolVersion::new(2, 2, 9));
    assert!(ToolVersion::new(2, 3, 1) > ToolVersion::new(2, 3, 0));
}

#[test]
fn a_tool_older_than_the_servers_minimum_is_too_old() {
    let server = ServerVersion::new(8, 0);
    assert_eq!(
        compatibility(ToolKind::Mongosh, ToolVersion::new(1, 10, 0), Some(server)),
        Compatibility::TooOld
    );
}

#[test]
fn a_recent_tool_is_compatible() {
    let server = ServerVersion::new(8, 0);
    assert_eq!(
        compatibility(ToolKind::Mongosh, ToolVersion::new(2, 5, 0), Some(server)),
        Compatibility::Ok
    );
    assert_eq!(
        compatibility(
            ToolKind::Mongodump,
            ToolVersion::new(100, 12, 0),
            Some(server)
        ),
        Compatibility::Ok
    );
}

#[test]
fn an_unknown_server_is_never_a_warning() {
    assert_eq!(
        compatibility(ToolKind::Mongorestore, ToolVersion::new(100, 0, 0), None),
        Compatibility::Unknown
    );
}

#[test]
fn a_server_older_than_the_table_has_no_minimum() {
    assert_eq!(
        compatibility(
            ToolKind::Mongodump,
            ToolVersion::new(100, 0, 0),
            Some(ServerVersion::new(3, 6))
        ),
        Compatibility::Ok
    );
}

#[test]
fn a_server_version_string_gives_major_and_minor() {
    assert_eq!(
        ServerVersion::parse("8.0.3"),
        Some(ServerVersion::new(8, 0))
    );
    assert_eq!(
        ServerVersion::parse("7.0.14-rc0"),
        Some(ServerVersion::new(7, 0))
    );
    assert_eq!(ServerVersion::parse("6.0"), Some(ServerVersion::new(6, 0)));
}

#[test]
fn a_server_version_that_is_not_a_number_is_none() {
    assert_eq!(ServerVersion::parse(""), None);
    assert_eq!(ServerVersion::parse("x.y.z"), None);
    assert_eq!(ServerVersion::parse("8"), None);
}
