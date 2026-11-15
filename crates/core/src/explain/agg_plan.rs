//! Reads a server aggregate explain document into an [`AggPlan`].
//!
//! Shapes handled (MongoDB 5 to 8): the `stages` array whose leading `$cursor` holds the pushed
//! down plan, the top-level `queryPlanner` of a pipeline the server pushed down entirely, a
//! sharded `shards` map, and the sharded `queryPlanner.winningPlan.shards` list. Anything else
//! (an error document, a missing plan) is [`AggPlanOutcome::NotUnderstood`]. The parser never
//! panics: every read is by key and a field of the wrong type reads as missing. Explain
//! documents hold user data and are never logged here.

use bson::{Bson, Document};

use super::agg_model::{
    AggHint, AggPlan, AggPlanOutcome, AggStage, AggSummary, ExplainVerbosity, InputPlan,
};
use super::agg_stage::{
    ParsedInput, ParsedStage, examined_hint, is_input_marker, opt_count, parse_input, parse_stage,
    stage_operator,
};

/// Parses the document `explain` answered for an aggregate.
pub fn parse_aggregate_explain(doc: &Document) -> AggPlanOutcome {
    if reports_failure(doc) {
        return AggPlanOutcome::NotUnderstood;
    }
    parse_document(doc).map_or(AggPlanOutcome::NotUnderstood, AggPlanOutcome::Plan)
}

fn reports_failure(doc: &Document) -> bool {
    match doc.get("ok") {
        Some(Bson::Int32(0) | Bson::Int64(0) | Bson::Boolean(false)) => true,
        Some(Bson::Double(n)) => *n == 0.0,
        _ => false,
    }
}

fn parse_document(doc: &Document) -> Option<AggPlan> {
    match doc.get_document("shards") {
        Ok(shards) => sharded_by_name(doc, shards),
        Err(_) => single(doc),
    }
}

/// An unsharded plan: the `stages` form, else the top-level `queryPlanner` form.
fn single(doc: &Document) -> Option<AggPlan> {
    let stats = doc.get_document("executionStats").ok();
    if let Ok(stages) = doc.get_array("stages") {
        return from_stages(stages, stats);
    }
    from_planner(doc.get_document("queryPlanner").ok()?, stats)
}

fn parse_stage_list(items: &[Bson]) -> Vec<ParsedStage> {
    items
        .iter()
        .filter_map(Bson::as_document)
        .filter(|d| stage_operator(d).is_some_and(|name| !is_input_marker(name)))
        .filter_map(parse_stage)
        .collect()
}

fn cursor_input(items: &[Bson]) -> Option<ParsedInput> {
    let cursor = items.iter().filter_map(Bson::as_document).find_map(|d| {
        let name = stage_operator(d).filter(|n| is_input_marker(n))?;
        d.get_document(name).ok()
    })?;
    let planner = cursor.get_document("queryPlanner").ok()?;
    parse_input(planner, cursor.get_document("executionStats").ok())
}

fn from_stages(items: &[Bson], stats: Option<&Document>) -> Option<AggPlan> {
    let input = cursor_input(items);
    let stages = parse_stage_list(items);
    (input.is_some() || !stages.is_empty()).then(|| assemble(input, stages, stats))
}

fn from_planner(planner: &Document, stats: Option<&Document>) -> Option<AggPlan> {
    let shard_entries = planner
        .get_document("winningPlan")
        .ok()
        .and_then(|w| w.get_array("shards").ok());
    match shard_entries {
        Some(entries) => sharded_by_list(entries, stats),
        None => parse_input(planner, stats).map(|input| assemble(Some(input), Vec::new(), stats)),
    }
}

/// The figure of `stats` for the shard called `name` in `executionStages.shards`.
fn shard_stats<'a>(stats: Option<&'a Document>, name: &str) -> Option<&'a Document> {
    stats?
        .get_document("executionStages")
        .ok()?
        .get_array("shards")
        .ok()?
        .iter()
        .filter_map(Bson::as_document)
        .find(|entry| entry.get_str("shardName") == Ok(name))
}

fn sharded_by_list(entries: &[Bson], stats: Option<&Document>) -> Option<AggPlan> {
    let shards: Vec<(String, AggPlan)> = entries
        .iter()
        .filter_map(Bson::as_document)
        .enumerate()
        .filter_map(|(position, entry)| {
            let name = entry
                .get_str("shardName")
                .map_or_else(|_| format!("#{}", position + 1), str::to_owned);
            let shard_stats = shard_stats(stats, &name);
            let input = parse_input(entry, shard_stats)?;
            Some((name, assemble(Some(input), Vec::new(), shard_stats)))
        })
        .collect();
    (!shards.is_empty()).then(|| combine(shards, Vec::new(), stats))
}

fn sharded_by_name(doc: &Document, shards: &Document) -> Option<AggPlan> {
    let plans: Vec<(String, AggPlan)> = shards
        .iter()
        .filter_map(|(name, value)| Some((name.clone(), single(value.as_document()?)?)))
        .collect();
    let merger = doc
        .get_document("splitPipeline")
        .ok()
        .and_then(|split| split.get_array("mergerPart").ok())
        .map(|items| parse_stage_list(items))
        .unwrap_or_default();
    (!plans.is_empty()).then(|| combine(plans, merger, doc.get_document("executionStats").ok()))
}

/// Folds `hints` into `into` without duplicates, keeping the first occurrence's position.
fn extend_unique(into: &mut Vec<AggHint>, hints: impl IntoIterator<Item = AggHint>) {
    for hint in hints {
        if !into.contains(&hint) {
            into.push(hint);
        }
    }
}

fn max_of(values: impl Iterator<Item = Option<u64>>) -> Option<u64> {
    values.flatten().max()
}

fn sum_of(values: impl Iterator<Item = Option<u64>>) -> Option<u64> {
    values.flatten().fold(None, |sum, n| {
        Some(sum.map_or(n, |total: u64| total.saturating_add(n)))
    })
}

fn verbosity(executed: bool) -> ExplainVerbosity {
    match executed {
        true => ExplainVerbosity::ExecutionStats,
        false => ExplainVerbosity::QueryPlanner,
    }
}

/// One plan from an input, the stages after it and the document-level statistics, if any.
fn assemble(
    input: Option<ParsedInput>,
    stages: Vec<ParsedStage>,
    stats: Option<&Document>,
) -> AggPlan {
    let stat = |key: &str| stats.and_then(|s| opt_count(s, key));
    let last_returned = stages.last().and_then(|s| s.stage.n_returned);
    let input_figure = |pick: fn(&InputPlan) -> Option<u64>| {
        input.as_ref().and_then(|i| pick(&i.plan))
    };
    let executed = stats.is_some()
        || input.as_ref().is_some_and(|i| i.has_stats)
        || stages
            .iter()
            .any(|s| s.stage.n_returned.is_some() || s.stage.millis.is_some());
    let n_returned = stat("nReturned").or(last_returned).or_else(|| match stages.is_empty() {
        true => input_figure(|i| i.n_returned),
        false => None,
    });
    let summary = AggSummary {
        verbosity_seen: verbosity(executed),
        total_millis: max_of(
            [stat("executionTimeMillis"), input.as_ref().and_then(|i| i.millis)]
                .into_iter()
                .chain(stages.iter().map(|s| s.stage.millis)),
        ),
        n_returned,
        keys_examined: stat("totalKeysExamined").or_else(|| input_figure(|i| i.keys_examined)),
        docs_examined: stat("totalDocsExamined").or_else(|| input_figure(|i| i.docs_examined)),
    };
    let mut hints = Vec::new();
    if let Some(parsed) = &input {
        extend_unique(&mut hints, parsed.hints.iter().copied());
        if parsed.has_stats {
            extend_unique(&mut hints, examined_hint(&parsed.plan));
        }
    }
    extend_unique(&mut hints, stages.iter().flat_map(|s| s.hints.iter().copied()));
    AggPlan {
        summary,
        input: input.map(|i| i.plan),
        stages: stages.into_iter().map(|s| s.stage).collect::<Vec<AggStage>>(),
        shards: Vec::new(),
        hints,
    }
}

/// The plan of a sharded explain: the plans of its shards under the merger stages.
fn combine(
    shards: Vec<(String, AggPlan)>,
    merger: Vec<ParsedStage>,
    stats: Option<&Document>,
) -> AggPlan {
    let stat = |key: &str| stats.and_then(|s| opt_count(s, key));
    let executed = stats.is_some()
        || shards
            .iter()
            .any(|(_, p)| p.summary.verbosity_seen == ExplainVerbosity::ExecutionStats);
    let summary = AggSummary {
        verbosity_seen: verbosity(executed),
        total_millis: stat("executionTimeMillis").or_else(|| {
            max_of(
                shards
                    .iter()
                    .map(|(_, p)| p.summary.total_millis)
                    .chain(merger.iter().map(|s| s.stage.millis)),
            )
        }),
        n_returned: stat("nReturned").or_else(|| merger.last().and_then(|s| s.stage.n_returned)),
        keys_examined: stat("totalKeysExamined")
            .or_else(|| sum_of(shards.iter().map(|(_, p)| p.summary.keys_examined))),
        docs_examined: stat("totalDocsExamined")
            .or_else(|| sum_of(shards.iter().map(|(_, p)| p.summary.docs_examined))),
    };
    let mut hints = Vec::new();
    for (_, plan) in &shards {
        extend_unique(&mut hints, plan.hints.iter().copied());
    }
    extend_unique(&mut hints, merger.iter().flat_map(|s| s.hints.iter().copied()));
    AggPlan {
        summary,
        input: None,
        stages: merger.into_iter().map(|s| s.stage).collect(),
        shards,
        hints,
    }
}

#[cfg(test)]
#[path = "agg_plan_tests.rs"]
mod tests;
