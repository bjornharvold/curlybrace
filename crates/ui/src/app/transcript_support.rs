//! Test support for the reducer transcripts: runs a script of inputs through `update` and renders
//! every step (the commands sent and a fixed projection of the state) as text, so a refactor of
//! the state model can be proven to change nothing observable.
//!
//! The text is stable between runs: session ids are renamed `S1`, `S2`... in order of first
//! appearance and the request numbers of commands are dropped (the script reads them from the
//! commands it was sent, so a wrong number would show as a different state, not as a different
//! number). The goldens live in `testdata/`; `UPDATE_GOLDEN=1` rewrites them.
use crate::app::reducer::{Input, update};
use crate::app::session::SessionView;
use crate::app::state::{AppState, ConnStatus, Pending};
use crate::viewmodel::{AggregateState, CollectionTab, IndexesState, SidebarState, ViewMode};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Command;
use curlybrace_core::store::{Page, Query};
use std::fmt::Write as _;
use std::path::Path;

/// Builds one step's input from the state before it and every command sent so far.
pub(crate) type Make = Box<dyn FnOnce(&AppState, &[Command]) -> Input>;
pub(crate) type Script = Vec<(&'static str, Make)>;

pub(crate) fn step(
    label: &'static str,
    make: impl FnOnce(&AppState, &[Command]) -> Input + 'static,
) -> (&'static str, Make) {
    (label, Box::new(make))
}

/// Runs `script` from `start`; the text of every step, with the final state's step last.
pub(crate) fn transcript(start: AppState, script: Script) -> String {
    let mut state = start;
    let mut sent: Vec<Command> = Vec::new();
    let mut text = String::new();
    for (label, make) in script {
        let input = make(&state, &sent);
        let (next, commands) = update(state, input);
        let _ = writeln!(text, "## {label}");
        for command in &commands {
            let _ = writeln!(text, "  send {}", command_line(command));
        }
        for line in projection(&next).lines() {
            let _ = writeln!(text, "  | {line}");
        }
        sent.extend(commands);
        state = next;
    }
    anonymised(&text)
}

/// Compares `actual` with `testdata/<name>`; `UPDATE_GOLDEN=1` writes it instead.
pub(crate) fn assert_golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/app/testdata")
        .join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("create testdata");
        std::fs::write(&path, actual).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing golden {}: {e} (UPDATE_GOLDEN=1 writes it)", name));
    assert_eq!(expected, actual, "transcript {name} changed");
}

fn command_line(command: &Command) -> String {
    match command {
        Command::Connect(cfg) => format!("Connect({})", cfg.id),
        other => without_numbers(&format!("{other:?}")),
    }
}

/// `text` without the `request: N` and `run: N` fields of the commands in it.
fn without_numbers(text: &str) -> String {
    let mut out = text.to_string();
    for key in ["request: ", "run: "] {
        while let Some(at) = out.find(key) {
            let digits = out[at + key.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .count();
            if digits == 0 {
                // Not a number (a type named in a field): leave it, but move past it.
                out.replace_range(at..at + key.len(), &key.replace(": ", ":\u{0}"));
                continue;
            }
            let end = at + key.len() + digits;
            if out[end..].starts_with(", ") {
                out.replace_range(at..end + 2, "");
            } else if out[..at].ends_with(", ") {
                out.replace_range(at - 2..end, "");
            } else {
                out.replace_range(at..end, "");
            }
        }
    }
    out.replace(":\u{0}", ": ")
}

/// Session ids renamed by first appearance.
fn anonymised(text: &str) -> String {
    let mut seen: Vec<String> = Vec::new();
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 36 <= bytes.len() && is_uuid(&text[i..i + 36]) {
            let id = text[i..i + 36].to_string();
            let n = match seen.iter().position(|s| *s == id) {
                Some(n) => n,
                None => {
                    seen.push(id);
                    seen.len() - 1
                }
            };
            let _ = write!(out, "S{}", n + 1);
            i += 36;
        } else {
            let ch = text[i..].chars().next().expect("a char");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn is_uuid(s: &str) -> bool {
    s.bytes().enumerate().all(|(i, b)| match i {
        8 | 13 | 18 | 23 => b == b'-',
        _ => b.is_ascii_hexdigit() && !b.is_ascii_uppercase(),
    })
}

/// What one session shows, read from the foreground fields or from a parked view alike.
struct Fields<'a> {
    status: ConnStatus,
    failure: &'a Option<CoreError>,
    sidebar: &'a SidebarState,
    loading: &'a std::collections::BTreeSet<String>,
    issued: &'a Query,
    shown: &'a Query,
    loading_page: bool,
    page: &'a Option<Page>,
    keyed: bool,
    tab: CollectionTab,
    view_mode: ViewMode,
    indexes: &'a IndexesState,
    aggregate: &'a AggregateState,
    editor: bool,
    pending: String,
    verdict: String,
    guard: String,
    find_text: String,
}

fn head(value: &dyn std::fmt::Debug) -> String {
    format!("{value:?}")
        .chars()
        .take_while(|c| c.is_alphanumeric())
        .collect()
}

/// The write in flight: its collection and whether it came from an editor save.
fn pending_of(pending: &Option<Pending>) -> String {
    pending.as_ref().map_or("none".to_string(), |p| {
        format!("{:?} editor={}", p.ns, p.editor.is_some())
    })
}

/// A query on one short line.
fn query_line(q: &Query) -> String {
    format!(
        "filter={:?} sort={:?} projection={:?} skip={} limit={}",
        q.filter, q.sort, q.projection, q.skip, q.limit
    )
}

fn of_state(s: &AppState) -> Fields<'_> {
    Fields {
        status: s.status,
        failure: &s.failure,
        sidebar: &s.sidebar,
        loading: &s.loading,
        issued: &s.issued_query,
        shown: &s.page_query,
        loading_page: s.loading_page,
        page: &s.page,
        keyed: s.page_key.is_some(),
        tab: s.tab,
        view_mode: s.view_mode,
        indexes: &s.indexes,
        aggregate: &s.aggregate,
        editor: s.editor.is_some(),
        pending: pending_of(&s.pending),
        verdict: head(&s.verdict),
        guard: head(&*s.guard),
        find_text: format!("{:?}", s.find.text),
    }
}

fn of_view(v: &SessionView) -> Fields<'_> {
    Fields {
        status: v.status,
        failure: &v.failure,
        sidebar: &v.sidebar,
        loading: &v.loading,
        issued: &v.issued_query,
        shown: &v.page_query,
        loading_page: v.loading_page,
        page: &v.page,
        keyed: v.page_key.is_some(),
        tab: v.tab,
        view_mode: v.view_mode,
        indexes: &v.indexes,
        aggregate: &v.aggregate,
        editor: v.editor.is_some(),
        pending: pending_of(&v.pending),
        verdict: head(&v.verdict),
        guard: head(&*v.guard),
        find_text: format!("{:?}", v.find.text),
    }
}

fn session_lines(f: &Fields<'_>) -> String {
    let dbs: Vec<String> = f
        .sidebar
        .databases
        .iter()
        .map(|d| {
            format!(
                "{}{}[{}]",
                d.name,
                if d.expanded { "+" } else { "" },
                d.collections.as_ref().map_or(0, Vec::len)
            )
        })
        .collect();
    let page = f.page.as_ref().map_or("none".to_string(), |p| {
        format!("{}docs skip={} more={}", p.docs.len(), p.skip, p.has_more)
    });
    let agg = f.aggregate;
    format!(
        "{status:?} failure={failure:?} databases={dbs:?} loading={loading:?} selected={sel:?}\n\
         issued={issued:?}\nshown={shown:?} loading_page={lp} page={page} keyed={keyed}\n\
         tab={tab:?} view={view:?} editor={editor} pending={pending} verdict={verdict} guard={guard}\n\
         find_text={find_text}\n\
         indexes ns={ins:?} loading={il} listed={n} series={ns_} selected={isel:?} writing={w} queries_loading={ql}\n\
         aggregate stages={stages} selected={asel} preview={preview} in_flight={fl} queued={q}",
        status = f.status,
        failure = f.failure,
        loading = f.loading,
        sel = f.sidebar.selected,
        issued = query_line(f.issued),
        shown = query_line(f.shown),
        lp = f.loading_page,
        keyed = f.keyed,
        tab = f.tab,
        view = f.view_mode,
        editor = f.editor,
        pending = f.pending,
        verdict = f.verdict,
        guard = f.guard,
        find_text = f.find_text,
        ins = f.indexes.ns,
        il = f.indexes.loading,
        n = f.indexes.indexes.len(),
        ns_ = f.indexes.series.len(),
        isel = f.indexes.selected,
        w = f.indexes.writing.is_some(),
        ql = f.indexes.queries.loading,
        stages = agg.stages.len(),
        asel = agg.selected,
        preview = head(&agg.preview),
        fl = agg.in_flight.is_some(),
        q = agg.queued.is_some(),
    )
}

/// A fixed text view of the state: the foreground session, every parked one, and the globals a
/// session event can touch.
pub(crate) fn projection(state: &AppState) -> String {
    let mut out = format!(
        "active={:?} open={:?} workspace={:?} banner={:?} banner_session={:?}\n",
        state.active, state.open, state.workspace, state.banner, state.banner_session
    );
    for id in &state.open {
        let lines = if state.active == Some(*id) {
            session_lines(&of_state(state))
        } else {
            match state.parked.get(id) {
                Some(view) => session_lines(&of_view(view)),
                None => "no view".to_string(),
            }
        };
        let which = if state.active == Some(*id) {
            "active"
        } else {
            "parked"
        };
        let _ = writeln!(out, "{id} ({which})");
        for line in lines.lines() {
            let _ = writeln!(out, "  {line}");
        }
    }
    let _ = writeln!(
        out,
        "write_warning={} admin={} bulk={}",
        state.write_warning.is_some(),
        state.admin.is_some(),
        state.bulk.is_some(),
    );
    out
}
