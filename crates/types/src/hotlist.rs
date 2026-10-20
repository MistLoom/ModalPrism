//! 热榜快照的投影，以及名次差和保留份数的变更集。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use strata_model::{
    Body, ChangeSet, EdgeDir, FacetKind, LineageInput, Op, Producer, Query, Rid, Target,
    content_hash,
};

use crate::article::{facet, facet_query};
use crate::error::Error;
use crate::read::{empty_args, text_field};
use crate::root::ReadRoot;

pub struct HotList;

impl HotList {
    pub const NAME: &'static str = "HotList";
    pub const VERSION: &'static str = "1";

    pub fn schema() -> Value {
        serde_json::from_str(include_str!("../../../config/schemas/hotlist.json"))
            .expect("hotlist schema")
    }

    pub fn selector() -> Query {
        facet_query("hotlist.snapshot")
    }

    pub async fn project(root: &ReadRoot, rid: &Rid) -> Result<Value, Error> {
        let Some(structure) = facet(
            root.strata(),
            rid,
            FacetKind::Struct,
            Some("hotlist.snapshot"),
            true,
        )
        .await?
        else {
            return Ok(Value::Null);
        };
        let relations = root.strata().related(rid, None, EdgeDir::Out).await?;
        let list = part_of_target(&relations);
        let mut entries = Vec::new();
        for relation in relations.iter().filter(|relation| relation.rel == "entry") {
            let rank = rank_of(relation.attrs.as_ref());
            entries.push(json!({
                "dst": relation.dst.to_string(),
                "rank": match rank {
                    Some(rank) => Value::from(rank),
                    None => Value::Null,
                },
                "attrs": relation.attrs.clone().unwrap_or(Value::Null),
            }));
        }
        entries.sort_by(cmp_entry);
        let delta = match facet(
            root.strata(),
            rid,
            FacetKind::Struct,
            Some("hotlist.delta"),
            true,
        )
        .await?
        {
            Some(facet) => facet.value.unwrap_or(Value::Null),
            None => Value::Null,
        };
        Ok(json!({
            "rid": rid.to_string(),
            "list": list,
            "snapshot": structure.value,
            "entries": entries,
            "delta": delta,
        }))
    }

    pub async fn on_commit(
        strata: &strata_client::Strata,
        applied: &config::Snapshot,
    ) -> Result<Value, Error> {
        match Self::changeset(strata, applied).await? {
            Some(changeset) => serde_json::to_value(changeset)
                .map_err(|source| Error::with_source(source.to_string(), source)),
            None => Ok(Value::Null),
        }
    }

    pub async fn changeset(
        strata: &strata_client::Strata,
        snapshot: &config::Snapshot,
    ) -> Result<Option<ChangeSet>, Error> {
        let page = strata.query(Self::selector()).await?;
        let mut groups: BTreeMap<String, Vec<Snapshot>> = BTreeMap::new();
        for row in page.rows {
            if row.get("tombstone").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let Some(rid_text) = row.get("rid").and_then(Value::as_str) else {
                continue;
            };
            let rid = Rid::parse(rid_text)
                .map_err(|source| Error::with_source(source.to_string(), source))?;
            let relations = strata.related(&rid, Some("part_of"), EdgeDir::Out).await?;
            let key = part_of_target(&relations).unwrap_or_else(|| rid.to_string());
            groups.entry(key).or_default().push(Snapshot {
                rid,
                created_at: row
                    .get("createdAt")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            });
        }
        let keep = keep_snapshots(snapshot);
        let mut ops = Vec::new();
        for group in groups.values_mut() {
            group.sort_by(|left, right| {
                cmp_created(&left.created_at, &right.created_at)
                    .then_with(|| left.rid.as_str().cmp(right.rid.as_str()))
            });
            if group.len() >= 2 {
                let previous_rid = group[group.len() - 2].rid.clone();
                let current_rid = group[group.len() - 1].rid.clone();
                let previous = entries_of(strata, &previous_rid).await?;
                let current = entries_of(strata, &current_rid).await?;
                let delta = delta_value(&previous, &current);
                let changed = delta["rankDelta"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
                    || delta["newEntries"]
                        .as_array()
                        .is_some_and(|items| !items.is_empty());
                if changed {
                    let existing = facet(
                        strata,
                        &current_rid,
                        FacetKind::Struct,
                        Some("hotlist.delta"),
                        true,
                    )
                    .await?;
                    let same = existing
                        .as_ref()
                        .and_then(|facet| facet.value.as_ref())
                        .is_some_and(|value| value == &delta);
                    if !same {
                        ops.push(Op::PutFacet {
                            target: Target::Rid(current_rid),
                            kind: FacetKind::Struct,
                            variant: Some("hotlist.delta".into()),
                            body: Body::Struct { value: delta },
                            media_type: Some("application/json".into()),
                            lineage: LineageInput {
                                inputs: Vec::new(),
                                params_hash: "hotlist.delta".into(),
                            },
                        });
                    }
                }
            }
            if let Some(keep) = keep {
                if group.len() > keep {
                    let end = group.len() - keep;
                    for older in &group[..end] {
                        ops.push(Op::Tombstone {
                            target: Target::Rid(older.rid.clone()),
                            reason: "keepSnapshots".into(),
                        });
                    }
                }
            }
        }
        if ops.is_empty() {
            return Ok(None);
        }
        let encoded = serde_json::to_vec(&ops)
            .map_err(|source| Error::with_source(source.to_string(), source))?;
        Ok(Some(ChangeSet {
            producer: Producer {
                module: "type_hotlist".into(),
                version: "1".into(),
                config_generation: snapshot.generation,
            },
            idempotency_key: Some(content_hash(&encoded)),
            ack: None,
            ops,
        }))
    }
}

struct Snapshot {
    rid: Rid,
    created_at: String,
}

struct Entry {
    dst: String,
    rank: Option<i64>,
}

pub(crate) async fn project_call(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    let rid = text_field(args, "rid")?;
    if args.as_object().is_some_and(|object| object.len() != 1) {
        return Err(Error::new("spec 无效"));
    }
    let rid = Rid::parse(&rid).map_err(|_| Error::new("spec 无效"))?;
    HotList::project(root, &rid).await
}

pub(crate) async fn on_commit_call(root: &ReadRoot, args: &Value) -> Result<Value, Error> {
    empty_args(args)?;
    HotList::on_commit(root.strata(), root.config().applied().as_ref()).await
}

fn keep_snapshots(snapshot: &config::Snapshot) -> Option<usize> {
    let value = snapshot
        .modules
        .get("type/hotlist")?
        .pointer("/spec/keepSnapshots")?;
    let count = value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))?;
    if count == 0 {
        None
    } else {
        usize::try_from(count).ok()
    }
}

fn part_of_target(relations: &[strata_client::Relation]) -> Option<String> {
    let mut targets: Vec<String> = relations
        .iter()
        .filter(|relation| relation.rel == "part_of")
        .map(|relation| relation.dst.to_string())
        .collect();
    targets.sort();
    targets.dedup();
    targets.into_iter().next()
}

fn rank_of(attrs: Option<&Value>) -> Option<i64> {
    let value = attrs?.get("rank")?;
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
}

async fn entries_of(strata: &strata_client::Strata, rid: &Rid) -> Result<Vec<Entry>, Error> {
    let relations = strata.related(rid, Some("entry"), EdgeDir::Out).await?;
    let mut entries: Vec<Entry> = relations
        .into_iter()
        .map(|relation| Entry {
            dst: relation.dst.to_string(),
            rank: rank_of(relation.attrs.as_ref()),
        })
        .collect();
    entries.sort_by(|left, right| left.dst.cmp(&right.dst).then(left.rank.cmp(&right.rank)));
    Ok(entries)
}

fn delta_value(previous: &[Entry], current: &[Entry]) -> Value {
    let previous_ranks = ranks(previous);
    let current_ranks = ranks(current);
    let mut rank_delta = Vec::new();
    for (dst, to) in &current_ranks {
        let Some(from) = previous_ranks.get(dst) else {
            continue;
        };
        let delta = from - to;
        if delta == 0 {
            continue;
        }
        rank_delta.push(json!({
            "dst": dst,
            "from": from,
            "to": to,
            "delta": delta,
        }));
    }
    let seen_before: BTreeSet<&str> = previous.iter().map(|entry| entry.dst.as_str()).collect();
    let mut seen = BTreeSet::new();
    let mut current_order: Vec<&Entry> = current.iter().collect();
    current_order.sort_by(|left, right| match (left.rank, right.rank) {
        (Some(left_rank), Some(right_rank)) => {
            left_rank.cmp(&right_rank).then(left.dst.cmp(&right.dst))
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => left.dst.cmp(&right.dst),
    });
    let mut new_entries = Vec::new();
    for entry in current_order {
        if seen_before.contains(entry.dst.as_str()) || !seen.insert(entry.dst.as_str()) {
            continue;
        }
        new_entries.push(Value::String(entry.dst.clone()));
    }
    json!({
        "rankDelta": rank_delta,
        "newEntries": new_entries,
    })
}

fn ranks(entries: &[Entry]) -> BTreeMap<String, i64> {
    let mut map = BTreeMap::new();
    for entry in entries {
        if let Some(rank) = entry.rank {
            map.entry(entry.dst.clone()).or_insert(rank);
        }
    }
    map
}

fn cmp_created(left: &str, right: &str) -> std::cmp::Ordering {
    match (
        strata_model::Timestamp::parse(left),
        strata_model::Timestamp::parse(right),
    ) {
        (Ok(left_time), Ok(right_time)) => left_time.cmp(&right_time),
        _ => left.cmp(right),
    }
}

fn cmp_entry(left: &Value, right: &Value) -> std::cmp::Ordering {
    let left_rank = left.get("rank").and_then(Value::as_i64);
    let right_rank = right.get("rank").and_then(Value::as_i64);
    let rank = match (left_rank, right_rank) {
        (Some(left_rank), Some(right_rank)) => left_rank.cmp(&right_rank),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    };
    rank.then_with(|| {
        left.get("dst")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(right.get("dst").and_then(Value::as_str).unwrap_or_default())
    })
}

#[cfg(test)]
mod tests {
    use super::cmp_created;

    #[test]
    fn fractional_timestamps_sort_by_instant() {
        assert_eq!(
            cmp_created("2020-01-01T00:00:00.9Z", "2020-01-01T00:00:00.95Z"),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            cmp_created("2020-01-01T00:00:00.95Z", "2020-01-01T00:00:00.9Z"),
            std::cmp::Ordering::Greater
        );
    }
}
