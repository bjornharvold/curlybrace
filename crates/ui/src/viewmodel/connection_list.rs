//! Pure view-model for the connection table: filtering, sorting, grouping and display text.
use curlybrace_core::connection::{AuthMechanism, ConnectionConfig};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortColumn {
    #[default]
    Name,
    Server,
    Security,
    LastConnected,
    LastModified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Sort {
    pub column: SortColumn,
    pub descending: bool,
}

impl Sort {
    pub fn toggled(self, column: SortColumn) -> Self {
        if self.column == column {
            Self {
                descending: !self.descending,
                ..self
            }
        } else {
            Self {
                column,
                descending: false,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Security {
    pub tls: bool,
    pub ssh: bool,
    pub auth: AuthMechanism,
}

pub fn security(cfg: &ConnectionConfig) -> Security {
    Security {
        tls: cfg.tls.enabled,
        ssh: cfg.ssh.is_some(),
        auth: cfg.auth,
    }
}

/// Matches the name, the folder and the host list. The raw uri is never searched so
/// credential-looking text can never be matched.
fn matches(cfg: &ConnectionConfig, needle: &str) -> bool {
    let has = |s: &str| s.to_lowercase().contains(needle);
    has(&cfg.name) || has(&servers(cfg)) || cfg.folder.as_deref().is_some_and(has)
}

/// Most recent first when ascending (oldest first when descending); a row without a timestamp
/// is last in both directions.
fn recent_first(a: Option<i64>, b: Option<i64>, descending: bool) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) if descending => x.cmp(&y),
        (Some(x), Some(y)) => y.cmp(&x),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// The full row order for `sort`: the column, then name (case-insensitive, then exact), then id,
/// so ties never flicker. Descending reverses everything except where a column pins rows (the
/// untimed rows of the time columns).
fn compare(a: &ConnectionConfig, b: &ConnectionConfig, sort: Sort) -> Ordering {
    let directed = |ord: Ordering| if sort.descending { ord.reverse() } else { ord };
    let primary = match sort.column {
        SortColumn::Name => Ordering::Equal,
        SortColumn::Server => directed(servers(a).to_lowercase().cmp(&servers(b).to_lowercase())),
        SortColumn::Security => {
            let key = |c: &ConnectionConfig| {
                (
                    c.tls.enabled,
                    c.ssh.is_some(),
                    c.auth != AuthMechanism::None,
                )
            };
            directed(key(a).cmp(&key(b)))
        }
        SortColumn::LastConnected => {
            recent_first(a.last_connected, b.last_connected, sort.descending)
        }
        SortColumn::LastModified => recent_first(a.updated_at, b.updated_at, sort.descending),
    };
    primary.then_with(|| {
        directed(
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.id.cmp(&b.id)),
        )
    })
}

pub fn visible<'a>(
    list: &'a [ConnectionConfig],
    filter: &str,
    sort: Sort,
) -> Vec<&'a ConnectionConfig> {
    let needle = filter.trim().to_lowercase();
    let mut rows: Vec<&ConnectionConfig> = list
        .iter()
        .filter(|c| needle.is_empty() || matches(c, &needle))
        .collect();
    rows.sort_by(|a, b| compare(a, b, sort));
    rows
}

pub struct Group<'a> {
    pub folder: Option<&'a str>,
    pub rows: Vec<&'a ConnectionConfig>,
}

/// Root first, then folders alphabetically (case-sensitive); row order is preserved and
/// empty groups are omitted.
pub fn grouped<'a>(rows: &[&'a ConnectionConfig]) -> Vec<Group<'a>> {
    let mut folders: Vec<&str> = rows.iter().filter_map(|c| c.folder.as_deref()).collect();
    folders.sort_unstable();
    folders.dedup();
    let mut groups = vec![Group {
        folder: None,
        rows: rows
            .iter()
            .copied()
            .filter(|c| c.folder.is_none())
            .collect(),
    }];
    groups.extend(folders.into_iter().map(|f| {
        Group {
            folder: Some(f),
            rows: rows
                .iter()
                .copied()
                .filter(|c| c.folder.as_deref() == Some(f))
                .collect(),
        }
    }));
    groups.retain(|g| !g.rows.is_empty());
    groups
}

/// The hosts of the URI, without credentials, path or options. Never panics; a uri that
/// is not a connection string yields whatever host-like text it has (possibly empty).
pub fn servers(cfg: &ConnectionConfig) -> String {
    curlybrace_core::connection_repair::uri_hosts(&cfg.uri)
}

const MIN_DISPLAY_SECS: i64 = -62_167_219_200; // 0000-01-01 00:00:00 UTC
const MAX_DISPLAY_SECS: i64 = 253_402_300_799; // 9999-12-31 23:59:59 UTC

/// `YYYY-MM-DD HH:MM` in UTC, or `-` outside years 0000..=9999.
pub fn format_utc(secs: i64) -> String {
    if !(MIN_DISPLAY_SECS..=MAX_DISPLAY_SECS).contains(&secs) {
        return "-".to_owned();
    }
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m) = (rem / 3600, rem % 3600 / 60);
    // civil-from-days; bounded inputs keep all arithmetic far from overflow
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{d:02} {h:02}:{m:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(name: &str, uri: &str, folder: Option<&str>) -> ConnectionConfig {
        let mut c = ConnectionConfig::new(name, uri);
        c.folder = folder.map(str::to_owned);
        c
    }

    #[test]
    fn filter_matches_name_folder_and_host_case_insensitively() {
        let list = [
            cfg("Prod", "mongodb://db.example.com", None),
            cfg("Stage", "mongodb://other", Some("Work")),
        ];
        let names = |f: &str| {
            visible(&list, f, Sort::default())
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names("prod"), vec!["Prod"]);
        assert_eq!(names("WORK"), vec!["Stage"]);
        assert_eq!(names("EXAMPLE"), vec!["Prod"], "matches the host");
        assert_eq!(
            names("  "),
            vec!["Prod", "Stage"],
            "blank filter shows everything"
        );
        assert!(names("nomatch").is_empty());
    }

    #[test]
    fn filter_never_matches_credential_text_in_the_stored_uri() {
        let list = [cfg("a", "mongodb://alice:s3cret@h1", None)];
        assert!(visible(&list, "s3cret", Sort::default()).is_empty());
        assert!(visible(&list, "alice", Sort::default()).is_empty());
        assert_eq!(visible(&list, "h1", Sort::default()).len(), 1);
    }

    #[test]
    fn sorting_by_name_ignores_case_and_can_descend() {
        let list = [cfg("b", "mongodb://h", None), cfg("A", "mongodb://h", None)];
        let asc = visible(&list, "", Sort::default());
        assert_eq!((asc[0].name.as_str(), asc[1].name.as_str()), ("A", "b"));
        let desc = visible(
            &list,
            "",
            Sort {
                column: SortColumn::Name,
                descending: true,
            },
        );
        assert_eq!(desc[0].name, "b");
    }

    fn timed(name: &str, last_connected: Option<i64>, updated_at: Option<i64>) -> ConnectionConfig {
        let mut c = cfg(name, "mongodb://h", None);
        c.last_connected = last_connected;
        c.updated_at = updated_at;
        c
    }

    fn order(list: &[ConnectionConfig], column: SortColumn, descending: bool) -> Vec<String> {
        visible(list, "", Sort { column, descending })
            .iter()
            .map(|c| c.name.clone())
            .collect()
    }

    #[test]
    fn untimed_rows_stay_last_in_both_directions_for_both_time_columns() {
        let list = [
            timed("never", None, None),
            timed("old", Some(10), Some(10)),
            timed("new", Some(30), Some(30)),
            timed("mid", Some(20), Some(20)),
            timed("never2", None, None),
        ];
        for column in [SortColumn::LastConnected, SortColumn::LastModified] {
            assert_eq!(
                order(&list, column, false),
                ["new", "mid", "old", "never", "never2"],
                "{column:?} ascending: most recent first, never last"
            );
            assert_eq!(
                order(&list, column, true),
                ["old", "mid", "new", "never2", "never"],
                "{column:?} descending: oldest first, never still last"
            );
        }
    }

    #[test]
    fn equal_timestamps_and_untimed_ties_are_deterministic() {
        let list = [
            timed("b", Some(5), None),
            timed("a", Some(5), None),
            timed("d", None, None),
            timed("c", None, None),
        ];
        assert_eq!(
            order(&list, SortColumn::LastConnected, false),
            ["a", "b", "c", "d"]
        );
        assert_eq!(
            order(&list, SortColumn::LastConnected, true),
            ["b", "a", "d", "c"]
        );
    }

    #[test]
    fn ties_break_by_name_then_id_and_sorting_is_deterministic() {
        let mut list = vec![
            cfg("same", "mongodb://h", None),
            cfg("same", "mongodb://h", None),
            cfg("Same", "mongodb://h", None),
        ];
        for column in [
            SortColumn::Name,
            SortColumn::Server,
            SortColumn::Security,
            SortColumn::LastConnected,
            SortColumn::LastModified,
        ] {
            for descending in [false, true] {
                let sort = Sort { column, descending };
                let first: Vec<_> = visible(&list, "", sort).iter().map(|c| c.id).collect();
                list.reverse();
                let second: Vec<_> = visible(&list, "", sort).iter().map(|c| c.id).collect();
                assert_eq!(first, second, "{column:?} descending={descending}");
            }
        }
    }

    #[test]
    fn toggling_the_same_column_flips_direction_and_a_new_column_resets_it() {
        let s = Sort::default().toggled(SortColumn::Name);
        assert!(s.descending);
        let s = s.toggled(SortColumn::Server);
        assert_eq!((s.column, s.descending), (SortColumn::Server, false));
    }

    #[test]
    fn groups_put_the_root_first_then_folders_alphabetically() {
        let list = [
            cfg("a", "mongodb://h", Some("zeta")),
            cfg("b", "mongodb://h", None),
            cfg("c", "mongodb://h", Some("alpha")),
        ];
        let rows: Vec<_> = list.iter().collect();
        let groups = grouped(&rows);
        let order: Vec<_> = groups.iter().map(|g| g.folder).collect();
        assert_eq!(order, vec![None, Some("alpha"), Some("zeta")]);
    }

    #[test]
    fn grouped_preserves_row_order_omits_empty_groups_and_is_case_sensitive() {
        let list = [
            cfg("1", "mongodb://h", Some("b")),
            cfg("2", "mongodb://h", Some("B")),
            cfg("3", "mongodb://h", Some("b")),
        ];
        let rows: Vec<_> = list.iter().collect();
        let groups = grouped(&rows);
        let order: Vec<_> = groups.iter().map(|g| g.folder).collect();
        assert_eq!(order, vec![Some("B"), Some("b")], "no empty root group");
        let names: Vec<_> = groups[1].rows.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["1", "3"]);
        assert!(grouped(&[]).is_empty());
    }

    #[test]
    fn servers_lists_every_host_without_credentials_or_options() {
        assert_eq!(
            servers(&cfg("a", "mongodb://u@h1:27017,h2:27018/db?x=1", None)),
            "h1:27017, h2:27018"
        );
        assert_eq!(
            servers(&cfg("a", "mongodb+srv://c0.example.com/db", None)),
            "c0.example.com"
        );
    }

    #[test]
    fn servers_handles_ipv6_unix_sockets_userinfo_and_garbage() {
        let s = |uri: &str| servers(&cfg("a", uri, None));
        assert_eq!(s("mongodb://[::1]:27017/db"), "[::1]:27017");
        assert_eq!(
            s("mongodb://%2Ftmp%2Fmongodb-27017.sock/db"),
            "%2Ftmp%2Fmongodb-27017.sock"
        );
        assert_eq!(s("mongodb://user@h:1"), "h:1");
        assert_eq!(s("mongodb://u:p@w@h:1/?x=a@b"), "h:1");
        assert_eq!(
            s("mongodb://u:p/w@h:1"),
            "h:1",
            "no password fragment leaks"
        );
        assert_eq!(s(""), "");
        assert_eq!(s("mongodb://"), "");
        assert_eq!(s("?@/,"), "");
    }

    #[test]
    fn security_reports_tls_ssh_and_auth() {
        let mut c = cfg("a", "mongodb://h", None);
        c.tls.enabled = true;
        c.auth = AuthMechanism::ScramSha256;
        let s = security(&c);
        assert!(s.tls && !s.ssh);
        assert_eq!(s.auth, AuthMechanism::ScramSha256);
    }

    #[test]
    fn format_utc_is_a_fixed_width_iso_like_stamp() {
        assert_eq!(format_utc(0), "1970-01-01 00:00");
        assert_eq!(format_utc(1_700_000_000), "2023-11-14 22:13");
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00");
    }

    #[test]
    fn format_utc_handles_negative_and_extreme_values_without_panicking() {
        assert_eq!(format_utc(-1), "1969-12-31 23:59");
        assert_eq!(format_utc(-86_400 * 365), "1969-01-01 00:00");
        assert_eq!(format_utc(253_402_300_799), "9999-12-31 23:59");
        assert_eq!(format_utc(-62_167_219_200), "0000-01-01 00:00");
        for v in [i64::MIN, i64::MAX, 253_402_300_800, -62_167_219_201] {
            assert_eq!(format_utc(v), "-");
        }
    }
}
