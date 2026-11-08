//! Tier 1 (local server): `read_profile` against a capped `system.profile` that rolls entries
//! off, and paging through more than one page. Writes only to throwaway `cbit_*` databases.
//! Run with `scripts/integration-tests.sh`.

mod support;

use bson::{Document, doc};
use curlybrace_core::store::PROFILE_READ_LIMIT;
use mongodb::{Client, Collection};
use support::local_enabled;
use support::profiler::{Env, collect, serial, start_mark, verb_of};

/// Recreates `system.profile` of `db` as a capped collection of `size` bytes (the profiler must
/// be off, or the collection cannot be dropped).
async fn recreate_profile_capped(client: &Client, db: &str, size: u64) {
    let d = client.database(db);
    let profile: Collection<Document> = d.collection("system.profile");
    profile.drop().await.unwrap();
    d.create_collection("system.profile")
        .capped(true)
        .size(size)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_roll_off_is_detected() {
    if !local_enabled("real_roll_off_is_detected") {
        return;
    }
    let _serial = serial().await;
    let env = Env::new().await;
    env.seed("orders", 10).await;
    recreate_profile_capped(&env.app, env.db(), 4096).await;
    let start = start_mark().await;
    env.level(2).await;
    for i in 0..3_i32 {
        collect(env.coll("orders").find(doc! { "b": i }).await.unwrap()).await;
    }
    let first = env.read(start).await;
    let watermark = first.newest_ts_millis.expect("entries were written");
    assert!(first.oldest_ts_millis.unwrap() <= watermark);
    // Far more than 4 KiB of entries before the next read.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    for i in 0..200_i32 {
        collect(env.coll("orders").find(doc! { "b": i % 10 }).await.unwrap()).await;
    }
    env.level(0).await;

    let second = env.read(watermark).await;
    let oldest = second.oldest_ts_millis.expect("the profile is not empty");
    assert!(
        oldest > watermark,
        "entries between the watermark ({watermark}) and {oldest} rolled off"
    );
    assert!(
        second.entries.len() < 200,
        "only the tail survived: {}",
        second.entries.len()
    );
    assert_eq!(second.skipped, 0);
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_profile_read_paging_with_more_than_a_page() {
    if !local_enabled("real_profile_read_paging_with_more_than_a_page") {
        return;
    }
    let _serial = serial().await;
    let env = Env::new().await;
    env.seed("orders", 5).await;
    // 16 MiB: the default 1 MiB would roll the early entries off before they are paged.
    recreate_profile_capped(&env.app, env.db(), 16 * 1024 * 1024).await;
    let total = PROFILE_READ_LIMIT + 250;
    let start = start_mark().await;
    env.level(2).await;
    for i in 0..total {
        collect(
            env.coll("orders")
                .find(doc! { "n": i as i64 })
                .await
                .unwrap(),
        )
        .await;
        // One entry per millisecond at most: a page boundary inside one millisecond may lose
        // the entries after it (the watermark is a strict `ts >`, an accepted limitation).
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    env.level(0).await;

    let orders = format!("{}.orders", env.db());
    let mut seen: Vec<i64> = Vec::new();
    let (mut watermark, mut pages) = (start, 0);
    loop {
        let page = env.read(watermark).await;
        assert_eq!(page.skipped, 0);
        pages += 1;
        let full = page.entries.len() as u64 + page.skipped >= PROFILE_READ_LIMIT as u64;
        if pages == 1 {
            assert!(
                full,
                "the first page is full: {} entries",
                page.entries.len()
            );
        }
        seen.extend(
            page.entries
                .iter()
                .filter(|e| e.ns == orders && verb_of(e) == "find")
                .filter_map(|e| e.command.get_document("filter").ok()?.get_i64("n").ok()),
        );
        let Some(newest) = page.newest_ts_millis.filter(|n| *n > watermark) else {
            break;
        };
        watermark = newest;
        assert!(pages < 20, "paging does not terminate");
    }
    assert!(pages >= 2, "{pages} pages");
    let mut sorted = seen.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(seen.len(), sorted.len(), "no operation was read twice");
    assert_eq!(
        sorted,
        (0..total as i64).collect::<Vec<_>>(),
        "every operation was read, none skipped"
    );
}
