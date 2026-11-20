//! Cutting a file that holds more than the bound, and how fast that is.
use super::*;

#[test]
fn a_file_over_the_bound_is_cut_the_way_the_running_app_cuts_it() {
    let dir = tempfile::tempdir().unwrap();
    let (b, c, d) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let mut tabs: Vec<SavedTab> = Vec::new();
    tabs.extend((0..150).map(|n| tab(b, &format!("b{n}"))));
    tabs.extend((0..70).map(|n| tab(c, &format!("c{n}"))));
    tabs.extend((0..30).map(|n| tab(d, &format!("d{n}"))));
    assert_eq!(tabs.len(), 250);
    let path = dir.path().join("workspace.json");
    std::fs::write(&path, serde_json::to_vec(&file_of(tabs)).unwrap()).unwrap();
    let tags = RefCell::new(Vec::new());
    let loaded = load(&path, &recording(&tags), &known(&[b, c, d]));
    let count = |id: Uuid| {
        loaded
            .file
            .tabs
            .iter()
            .filter(|t| t.connection == id)
            .count()
    };
    assert_eq!(
        (count(b), count(c), count(d)),
        (100, 70, 30),
        "the longest group lost its tail"
    );
    assert_eq!(loaded.file.tabs.len(), MAX_SAVED_TABS);
    assert_eq!(
        loaded.recovery,
        Some(Recovery::WorkspaceOverflow {
            count: 50,
            backed_up: true
        })
    );
    assert_eq!(*tags.borrow(), ["skipped"]);
}

#[test]
fn a_tab_repaired_at_an_earlier_launch_is_not_counted_again() {
    let dir = tempfile::tempdir().unwrap();
    let (kept, gone) = (
        "00000000-0000-0000-0000-000000000001",
        "00000000-0000-0000-0000-000000000002",
    );
    let flagged = format!(
        r#"{{"connection":"{kept}","db":"a","collection":"one","sub_tab":"documents","view_mode":"table","options_dropped":true}}"#
    );
    let bad = format!(
        r#"{{"connection":"{gone}","db":"a","collection":"two","sub_tab":"documents","view_mode":"table",
          "find":{{"filter":"{{}}","sort":"","projection":"","skip":"","limit":"","options":{{"maxTimeMS":0}}}}}}"#
    );
    let text = format!(r#"{{"version":1,"restore":true,"tabs":[{flagged},{bad}]}}"#);
    let path = write(&dir, text.as_bytes());
    let tags = RefCell::new(Vec::new());
    let id: Uuid = kept.parse().unwrap();
    let loaded = load(&path, &recording(&tags), &known(&[id]));
    assert_eq!(loaded.file.tabs.len(), 1);
    assert_eq!(
        loaded.recovery,
        Some(Recovery::WorkspaceSkipped {
            count: 1,
            backed_up: true
        })
    );
    assert!(
        loaded.also.is_empty(),
        "nothing was repaired by this load: {:?}",
        loaded.also
    );
    assert_eq!(*tags.borrow(), ["skipped"]);
}

/// The rule worked out one entry at a time: the last entry of the longest group goes, the larger
/// id first among equals.
fn one_at_a_time(
    sizes: &std::collections::BTreeMap<Uuid, usize>,
    cut: usize,
) -> std::collections::BTreeMap<Uuid, usize> {
    let mut left = sizes.clone();
    let mut cuts = std::collections::BTreeMap::new();
    for _ in 0..cut.min(sizes.values().sum()) {
        let (id, _) = left.iter().max_by_key(|(id, n)| (**n, **id)).unwrap();
        let id = *id;
        *left.get_mut(&id).unwrap() -= 1;
        *cuts.entry(id).or_default() += 1;
    }
    cuts
}

#[test]
fn the_one_pass_cut_is_the_one_at_a_time_cut() {
    let mut seed: u64 = 12345;
    let mut next = |n: u64| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };
    for _ in 0..400 {
        let groups = 1 + next(8);
        let sizes: std::collections::BTreeMap<Uuid, usize> = (0..groups)
            .map(|g| (Uuid::from_u128(u128::from(g) + 1), next(14) as usize))
            .collect();
        let total: usize = sizes.values().sum();
        let cut = next(total as u64 + 2) as usize;
        let fast = crate::app::workspace_read::cut_counts(&sizes, cut);
        let slow = one_at_a_time(&sizes, cut);
        assert_eq!(fast, slow, "sizes {sizes:?} cut {cut}");
    }
}

#[test]
fn a_file_of_tens_of_thousands_of_entries_loads_fast_and_is_cut_the_same_way() {
    let dir = tempfile::tempdir().unwrap();
    let ids: Vec<Uuid> = (1..=50).map(Uuid::from_u128).collect();
    // Groups of very different lengths: 55 000 entries in all.
    let mut text = String::from(r#"{"version":1,"restore":true,"tabs":["#);
    let mut sizes = std::collections::BTreeMap::new();
    for i in 0..55_000_usize {
        let id = ids[(i * i / 7 + i) % ids.len() % (1 + i % ids.len())];
        *sizes.entry(id).or_insert(0_usize) += 1;
        if i > 0 {
            text.push(',');
        }
        text.push_str(&format!(
            r#"{{"connection":"{id}","db":"a","collection":"c{i}","sub_tab":"documents","view_mode":"table"}}"#
        ));
    }
    text.push_str("]}");
    assert!(text.len() < MAX_FILE_BYTES, "{} bytes", text.len());
    let path = write(&dir, text.as_bytes());
    let known: BTreeSet<Uuid> = ids.iter().copied().collect();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let loaded = load(&path, &failing_ok, &known);
        let _ = tx.send((started.elapsed(), loaded));
    });
    let (took, loaded) = rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("loading 55 000 entries took more than 20 s");
    // Alone in a debug build this takes about a second (reading the JSON is most of it); the
    // quadratic cut took minutes. The bound is loose so a busy machine does not fail it.
    eprintln!("loading 55 000 entries took {took:?}");
    assert!(took < std::time::Duration::from_secs(8), "took {took:?}");
    assert_eq!(loaded.file.tabs.len(), MAX_SAVED_TABS);
    let expected = one_at_a_time(&sizes, 55_000 - MAX_SAVED_TABS);
    for (id, n) in &sizes {
        let kept = loaded
            .file
            .tabs
            .iter()
            .filter(|t| t.connection == *id)
            .count();
        assert_eq!(kept, n - expected.get(id).copied().unwrap_or(0), "{id}");
    }
}

fn failing_ok(path: &Path, _: &str) -> std::io::Result<PathBuf> {
    Ok(path.to_path_buf())
}

#[test]
fn many_big_tabs_never_make_a_file_too_big_to_write() {
    let dir = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let text_of = |n: usize| SavedFind {
        filter: "x".repeat(n),
        ..SavedFind::default()
    };
    let mut tabs: Vec<SavedTab> = (0..36)
        .map(|n| SavedTab {
            find: Some(text_of(250 * 1024 - n)),
            ..tab(id, &format!("big{n}"))
        })
        .collect();
    tabs.extend((0..4).map(|n| SavedTab {
        find: Some(text_of(1024)),
        ..tab(id, &format!("small{n}"))
    }));
    let mut s = AppState::default();
    s.restore.dormant.insert(id, tabs);
    let file = snapshot(&s);
    let path = dir.path().join("workspace.json");
    save(&path, &PersistGuard::default(), &file).expect("a write never fails on size");
    assert!(std::fs::metadata(&path).unwrap().len() <= MAX_FILE_BYTES as u64);
    let dropped: Vec<&SavedTab> = file.tabs.iter().filter(|t| t.text_dropped).collect();
    assert!(!dropped.is_empty(), "something had to go");
    assert!(
        dropped
            .iter()
            .all(|t| t.collection.starts_with("big") && t.find.is_none())
    );
    let small: Vec<&SavedTab> = file
        .tabs
        .iter()
        .filter(|t| t.collection.starts_with("small"))
        .collect();
    assert!(
        small.iter().all(|t| t.find.is_some() && !t.text_dropped),
        "small tabs keep their text"
    );
    // The largest went first: every tab that kept its text is smaller than every one that lost it.
    let number = |tab: &SavedTab| {
        tab.collection
            .trim_start_matches("big")
            .parse::<usize>()
            .unwrap()
    };
    let largest_kept = file
        .tabs
        .iter()
        .filter(|t| t.collection.starts_with("big") && t.find.is_some())
        .map(|t| 250 * 1024 - number(t))
        .max();
    let smallest_dropped = dropped
        .iter()
        .map(|t| 250 * 1024 - number(t))
        .min()
        .unwrap();
    assert!(
        largest_kept.is_none_or(|kept| kept <= smallest_dropped),
        "kept {largest_kept:?} must not be larger than dropped {smallest_dropped}"
    );
    assert_eq!(loaded_len(&path), file.tabs.len());
}

fn loaded_len(path: &Path) -> usize {
    let bytes = std::fs::read(path).unwrap();
    serde_json::from_slice::<WorkspaceFile>(&bytes)
        .unwrap()
        .tabs
        .len()
}

fn fits(file: &WorkspaceFile, dir: &tempfile::TempDir) -> u64 {
    let path = dir.path().join("workspace.json");
    save(&path, &PersistGuard::default(), file).expect("a write never fails on size");
    let size = std::fs::metadata(&path).unwrap().len();
    assert!(size <= MAX_FILE_BYTES as u64, "{size} bytes");
    size
}

fn stage(text: String) -> SavedStage {
    SavedStage {
        text,
        enabled: true,
    }
}

#[test]
fn many_empty_stages_and_a_few_control_character_bars_still_fit() {
    // 196 tabs of 200 empty stages: little text, a great deal of structure; and four find bars
    // of control characters, each of which takes six bytes in the file.
    let dir = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let mut tabs: Vec<SavedTab> = (0..196)
        .map(|n| SavedTab {
            find: None,
            draft: Some(SavedDraftText {
                stages: (0..200).map(|_| stage(String::new())).collect(),
                ..SavedDraftText::default()
            }),
            ..tab(id, &format!("stages{n}"))
        })
        .collect();
    tabs.extend((0..4).map(|n| SavedTab {
        find: Some(SavedFind {
            filter: "\u{1}".repeat(240 * 1024),
            ..SavedFind::default()
        }),
        ..tab(id, &format!("bar{n}"))
    }));
    let mut s = AppState::default();
    s.restore.dormant.insert(id, tabs);
    let file = snapshot(&s);
    fits(&file, &dir);
    assert!(
        file.tabs.iter().any(|t| t.text_dropped),
        "something had to go"
    );
}

#[test]
fn the_file_is_measured_the_way_it_is_written() {
    // 200 tabs of 200 stages of 132 bytes: measured on its own the tabs fit, nested in the file
    // under "tabs" (two more spaces on every line) they do not.
    let dir = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let tabs: Vec<SavedTab> = (0..200)
        .map(|n| SavedTab {
            find: None,
            draft: Some(SavedDraftText {
                stages: (0..200).map(|_| stage("y".repeat(132))).collect(),
                ..SavedDraftText::default()
            }),
            ..tab(id, &format!("deep{n}"))
        })
        .collect();
    let mut s = AppState::default();
    s.restore.dormant.insert(id, tabs);
    let file = snapshot(&s);
    fits(&file, &dir);
}

#[test]
fn a_file_that_cannot_be_made_to_fit_by_dropping_text_is_cut_not_left_unwritable() {
    let dir = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    // No text to drop: the names alone are over the bound.
    let tabs: Vec<SavedTab> = (0..200)
        .map(|n| SavedTab {
            find: None,
            collection: format!("{n}{}", "c".repeat(50_000)),
            ..tab(id, "x")
        })
        .collect();
    let mut s = AppState::default();
    s.restore.dormant.insert(id, tabs);
    let file = snapshot(&s);
    fits(&file, &dir);
    assert!(!file.tabs.is_empty() && file.tabs.len() < 200);
    assert!(file.active.is_none_or(|at| at < file.tabs.len()));
}

#[test]
fn a_snapshot_well_under_the_bound_is_not_measured() {
    let id = Uuid::new_v4();
    let tabs: Vec<SavedTab> = (0..40)
        .map(|n| SavedTab {
            find: Some(SavedFind {
                filter: "x".repeat(50 * 1024),
                ..SavedFind::default()
            }),
            ..tab(id, &format!("t{n}"))
        })
        .collect();
    let mut s = AppState::default();
    s.restore.dormant.insert(id, tabs);
    let before = crate::app::workspace_snapshot::MEASURED.with(|c| c.get());
    let file = snapshot(&s);
    assert_eq!(file.tabs.len(), 40);
    assert_eq!(
        crate::app::workspace_snapshot::MEASURED.with(|c| c.get()),
        before,
        "2 MiB of text is known to fit without serialising it"
    );
}

/// A hint of `keys` key-pattern entries: one line per key once the file is indented.
fn big_hint(keys: usize) -> curlybrace_core::pipeline_options::PipelineOptions {
    use curlybrace_core::pipeline_options::{HintOption, PipelineOptions};
    let mut doc = bson::Document::new();
    for i in 0..keys {
        doc.insert(format!("f{i}"), 1_i32);
    }
    PipelineOptions {
        hint: Some(HintOption::Keys(doc)),
        ..PipelineOptions::default()
    }
}

#[test]
fn the_options_of_a_draft_count_toward_the_size_of_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let tabs: Vec<SavedTab> = (0..14)
        .map(|n| SavedTab {
            find: None,
            draft: Some(SavedDraftText {
                options: big_hint(23_000),
                ..SavedDraftText::default()
            }),
            ..tab(id, &format!("hint{n}"))
        })
        .collect();
    let mut s = AppState::default();
    s.restore.dormant.insert(id, tabs);
    let file = snapshot(&s);
    fits(&file, &dir);
    assert!(
        file.tabs.iter().any(|t| t.text_dropped),
        "something had to go"
    );
}

#[test]
fn the_options_of_a_find_bar_are_counted_at_their_indented_size() {
    let dir = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    // Compact the options take about 3.7 MiB: they are under the bound as they are, indented
    // they are over it.
    let tabs: Vec<SavedTab> = (0..15)
        .map(|n| SavedTab {
            find: Some(SavedFind {
                options: big_hint(23_000),
                ..SavedFind::default()
            }),
            ..tab(id, &format!("hint{n}"))
        })
        .collect();
    let mut s = AppState::default();
    s.restore.dormant.insert(id, tabs);
    let file = snapshot(&s);
    fits(&file, &dir);
}
