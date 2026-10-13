//! Helpers for tier-1 compare/apply flows: drive the runtime through `Command::Compare`.
#![allow(dead_code)]

use bson::{Bson, Document, doc};
use curlybrace_core::compare::apply::{ApplyKind, ApplyOp, ApplyReport, Direction, IdPolicy};
use curlybrace_core::compare::engine::Summary;
use curlybrace_core::compare::key::{KeySpec, KeyTuple};
use curlybrace_core::compare::protocol::{
    CompareCommand, CompareEvent, PairId, PairOutcome, PairSpec,
};
use curlybrace_core::compare::results::Category;
use curlybrace_core::compare::rules::CompareRules;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::{Command, RuntimeHandle};
use curlybrace_core::store::Namespace;
use futures::TryStreamExt;

use super::next_compare;

pub const SESSION: u64 = 1;

pub fn send(rt: &RuntimeHandle, cmd: CompareCommand) {
    rt.tx.send(Command::Compare(cmd)).unwrap();
}

/// Opens session 1 with both sides on the same server.
pub async fn open(rt: &mut RuntimeHandle, config: &ConnectionConfig) {
    send(
        rt,
        CompareCommand::Open {
            session: SESSION,
            source: config.clone(),
            target: config.clone(),
        },
    );
    assert_eq!(
        next_compare(rt).await,
        CompareEvent::Opened { session: SESSION }
    );
}

pub fn pair(id: PairId, source: Namespace, target: Namespace, key: &[&str]) -> PairSpec {
    let key = KeySpec::new(key.iter().map(|s| s.to_string()).collect()).unwrap();
    let mut rules = CompareRules::for_key(&key);
    rules.include_id = key.contains_id();
    PairSpec {
        id,
        source,
        target,
        source_filter: doc! {},
        target_filter: doc! {},
        rules,
        key,
    }
}

/// Runs a compare of `pairs` to the end; returns each completed pair's summary.
pub async fn compare(
    rt: &mut RuntimeHandle,
    run: u64,
    pairs: Vec<PairSpec>,
) -> Vec<(PairId, Summary)> {
    send(
        rt,
        CompareCommand::Start {
            session: SESSION,
            run,
            pairs,
        },
    );
    let mut done = Vec::new();
    loop {
        match next_compare(rt).await {
            CompareEvent::PairDone {
                pair,
                outcome: PairOutcome::Completed(s),
                ..
            } => done.push((pair, s)),
            CompareEvent::PairDone { outcome, .. } => panic!("pair did not complete: {outcome:?}"),
            CompareEvent::RunDone { .. } => return done,
            CompareEvent::Progress { .. } => {}
            other => panic!("{other:?}"),
        }
    }
}

/// All keys of one category of a pair, paged by 1000.
pub async fn keys(rt: &mut RuntimeHandle, pair: PairId, category: Category) -> Vec<KeyTuple> {
    let mut out = Vec::new();
    loop {
        send(
            rt,
            CompareCommand::Rows {
                session: SESSION,
                pair,
                category,
                offset: out.len(),
                limit: 1000,
            },
        );
        match next_compare(rt).await {
            CompareEvent::Rows { total, rows, .. } => {
                out.extend(rows.into_iter().map(|r| r.key));
                if out.len() >= total {
                    return out;
                }
            }
            other => panic!("{other:?}"),
        }
    }
}

pub fn key(v: impl Into<Bson>) -> KeyTuple {
    KeyTuple(vec![v.into()])
}

pub fn op(pair: PairId, k: KeyTuple, kind: ApplyKind) -> ApplyOp {
    ApplyOp {
        pair,
        key: k,
        kind,
        direction: Direction::SourceToTarget,
    }
}

/// What an Apply produced: the progress events seen, per-pair reports in order.
pub struct Applied {
    pub reports: Vec<(PairId, ApplyReport)>,
}

/// Sends an Apply and collects reports until `ApplyDone`. `Failed` panics (use [`apply_refused`]).
pub async fn apply(
    rt: &mut RuntimeHandle,
    run: u64,
    ops: Vec<ApplyOp>,
    id_policy: IdPolicy,
) -> Applied {
    send(
        rt,
        CompareCommand::Apply {
            session: SESSION,
            run,
            ops,
            id_policy,
        },
    );
    let mut reports = Vec::new();
    loop {
        match next_compare(rt).await {
            CompareEvent::ApplyPairDone { pair, report, .. } => reports.push((pair, report)),
            CompareEvent::ApplyDone { .. } => return Applied { reports },
            CompareEvent::ApplyProgress { .. } => {}
            other => panic!("{other:?}"),
        }
    }
}

pub async fn all_docs(client: &mongodb::Client, ns: &Namespace) -> Vec<Document> {
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .find(doc! {})
        .sort(doc! { "_id": 1 })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

pub fn summary_tuple(s: &Summary) -> (u64, u64, u64, u64) {
    (s.equal, s.only_in_source, s.only_in_target, s.different)
}
