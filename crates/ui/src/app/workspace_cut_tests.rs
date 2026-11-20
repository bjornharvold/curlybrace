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
