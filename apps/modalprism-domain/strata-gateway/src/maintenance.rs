//! 保留策略：按快照与版本下界裁掉过期行。
//!
//! 只做删除，不改写最新一条快照，也不动 `g_identities`：身份表是解析入口，
//! 少一行就会让内容变成另一个资源。`dry_run` 只统计不写，供控制台先预览。

use std::collections::BTreeMap;

use serde::Serialize;
use strata_model::Timestamp;

use crate::data;
use crate::db::Db;
use crate::error::{Error, Result};
use crate::sql::Sql;

/// 一次清理的输入。字段为 `None` 表示这一项不清理。
#[derive(Clone, Debug, Default)]
pub struct PrunePlan {
    /// 早于这个时刻的快照行可以删；最新一条始终保留。
    pub snapshots_before: Option<Timestamp>,
    /// 每个 `(rid, kind, variant)` 保留最后几版；键是 facet 种类，`default` 是兜底。
    pub facet_keep: BTreeMap<String, u64>,
    /// 早于这个时刻且已无 facet 引用的 blob 行可以删。
    pub raw_before: Option<Timestamp>,
}

/// 逐表删除计数。
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PruneReport {
    pub dry_run: bool,
    /// 每张表被裁到的最小版本。
    pub floor: BTreeMap<String, u64>,
    pub snapshots: u64,
    pub resources: u64,
    pub resource_marks: u64,
    pub facets: u64,
    pub relations: u64,
    pub state: u64,
    pub blobs: u64,
    pub vectors: u64,
    pub text: u64,
}

/// 各表当前行数，给控制台的「保留与清理」卡片用。
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreCounts {
    pub resources: u64,
    pub marks: u64,
    pub facets: u64,
    pub blobs: u64,
    pub relations: u64,
    pub state: u64,
    pub snapshots: u64,
    pub tasks: u64,
}

/// 版本键与物理表的对应关系。版本键取自 `g_versions.name`。
const TABLES: &[(&str, &str)] = &[
    ("resources", "g_resources"),
    ("resource_marks", "g_resource_marks"),
    ("facets", "g_facets"),
    ("relations", "g_relations"),
    ("state", "g_state"),
    ("blobs", "g_blobs"),
    ("vectors", "g_vectors"),
    ("text", "g_fts"),
];

/// 按计划清理。返回逐表删除行数；`dry_run` 时为「将删行数」。
pub async fn prune(db: &Db, plan: &PrunePlan, dry_run: bool) -> Result<PruneReport> {
    let mut report = PruneReport {
        dry_run,
        ..Default::default()
    };
    let snapshots = data::snapshots(db).await?;
    let newest = snapshots.iter().map(|row| row.seq).max();
    let doomed = expired(&snapshots, newest, plan.snapshots_before)?;
    report.snapshots = doomed.len() as u64;
    if !dry_run && !doomed.is_empty() {
        let mut sql = Sql::new(db.dialect());
        sql.push("DELETE FROM g_snapshots WHERE snapshot_seq IN (");
        for (index, seq) in doomed.iter().enumerate() {
            if index > 0 {
                sql.push(", ");
            }
            sql.u64(*seq);
        }
        sql.push(")");
        report.snapshots = db.exec(&sql).await?;
    }
    let kept: Vec<_> = snapshots
        .iter()
        .filter(|row| !doomed.contains(&row.seq))
        .collect();
    let floors = floors(&kept);
    report.floor = floors
        .iter()
        .map(|(name, version)| ((*name).to_string(), *version))
        .collect();
    for (name, table) in TABLES {
        let Some(floor) = floors.get(name).copied().filter(|floor| *floor > 0) else {
            continue;
        };
        let removed = act(
            db,
            table,
            |sql| {
                sql.push("v < ");
                sql.u64(floor);
            },
            dry_run,
        )
        .await?;
        set(&mut report, name, removed);
    }
    let removed = facets(
        db,
        plan,
        floors.get("facets").copied().unwrap_or(0),
        dry_run,
    )
    .await?;
    report.facets = report.facets.saturating_add(removed);
    let removed = unreferenced(db, plan.raw_before.as_ref(), dry_run).await?;
    report.blobs = report.blobs.saturating_add(removed);
    Ok(report)
}

/// 各表行数。
pub async fn counts(db: &Db) -> Result<StoreCounts> {
    Ok(StoreCounts {
        resources: count(db, "g_resources").await?,
        marks: count(db, "g_resource_marks").await?,
        facets: count(db, "g_facets").await?,
        blobs: count(db, "g_blobs").await?,
        relations: count(db, "g_relations").await?,
        state: count(db, "g_state").await?,
        snapshots: count(db, "g_snapshots").await?,
        tasks: count(db, "g_tasks").await?,
    })
}

fn set(report: &mut PruneReport, name: &str, value: u64) {
    match name {
        "resources" => report.resources = value,
        "resource_marks" => report.resource_marks = value,
        "facets" => report.facets = value,
        "relations" => report.relations = value,
        "state" => report.state = value,
        "blobs" => report.blobs = value,
        "vectors" => report.vectors = value,
        "text" => report.text = value,
        _ => {}
    }
}

/// 过期快照：早于截止时刻，且不是最新一条。
fn expired(
    snapshots: &[data::SnapshotRow],
    newest: Option<u64>,
    before: Option<Timestamp>,
) -> Result<Vec<u64>> {
    let Some(before) = before else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for row in snapshots {
        if Some(row.seq) == newest {
            continue;
        }
        if Timestamp::parse(&row.published_at)? < before {
            out.push(row.seq);
        }
    }
    Ok(out)
}

/// 每张表在剩余快照里的最小版本，也就是还能读到的下界。
fn floors(snapshots: &[&data::SnapshotRow]) -> BTreeMap<&'static str, u64> {
    let mut out: BTreeMap<&'static str, u64> = BTreeMap::new();
    for row in snapshots {
        for (name, _) in TABLES {
            let Some(version) = row.versions.get(*name) else {
                continue;
            };
            let entry = out.entry(name).or_insert(*version);
            if version < entry {
                *entry = *version;
            }
        }
    }
    out
}

/// 每 `(rid, kind, variant)` 保留最后 N 版。显式键按 facet 种类生效，`default` 管其余种类。
async fn facets(db: &Db, plan: &PrunePlan, floor: u64, dry_run: bool) -> Result<u64> {
    let mut explicit = Vec::new();
    let mut fallback = None;
    for (name, keep) in &plan.facet_keep {
        if name == "default" {
            fallback = Some(*keep);
        } else {
            explicit.push((name.clone(), *keep));
        }
    }
    let mut total = 0;
    for (kind, keep) in &explicit {
        let removed = act(
            db,
            "g_facets",
            |sql| {
                sql.push("kind = ");
                sql.text(kind.clone());
                version_tail(sql, floor, *keep);
            },
            dry_run,
        )
        .await?;
        total += removed;
    }
    if let Some(keep) = fallback {
        let removed = act(
            db,
            "g_facets",
            |sql| {
                sql.push("1 = 1");
                if !explicit.is_empty() {
                    sql.push(" AND kind NOT IN (");
                    for (index, (kind, _)) in explicit.iter().enumerate() {
                        if index > 0 {
                            sql.push(", ");
                        }
                        sql.text(kind.clone());
                    }
                    sql.push(")");
                }
                version_tail(sql, floor, keep);
            },
            dry_run,
        )
        .await?;
        total += removed;
    }
    Ok(total)
}

/// 版本尾巴：只在 `floor` 之上裁，且保留最新 `keep` 版。
fn version_tail(sql: &mut Sql, floor: u64, keep: u64) {
    sql.push(" AND v >= ");
    sql.u64(floor);
    sql.push(" AND version <= (SELECT MAX(f.version) FROM g_facets f WHERE f.rid = g_facets.rid AND f.kind = g_facets.kind AND f.variant = g_facets.variant) - ");
    sql.u64(keep);
}

/// 已无 facet 引用的 blob 行；给了截止时刻就只删更早的。
async fn unreferenced(db: &Db, before: Option<&Timestamp>, dry_run: bool) -> Result<u64> {
    let before = before.map(|value| value.to_rfc3339()).transpose()?;
    act(
        db,
        "g_blobs",
        |sql| {
            sql.push("NOT EXISTS (SELECT 1 FROM g_facets f WHERE f.blob_ref = g_blobs.sha256)");
            if let Some(before) = &before {
                sql.push(" AND created_at < ");
                sql.text(before.clone());
            }
        },
        dry_run,
    )
    .await
}

/// `dry_run` 时只数，否则删。两方言同形。
async fn act(db: &Db, table: &str, condition: impl Fn(&mut Sql), dry_run: bool) -> Result<u64> {
    let prefix = if dry_run {
        format!("SELECT COUNT(*) AS n FROM {table} WHERE (")
    } else {
        format!("DELETE FROM {table} WHERE (")
    };
    let mut sql = Sql::new(db.dialect());
    sql.push(prefix);
    condition(&mut sql);
    sql.push(")");
    if dry_run {
        Ok(db
            .fetch_int(&sql)
            .await?
            .map(|value| value.max(0) as u64)
            .unwrap_or(0))
    } else {
        db.exec(&sql).await
    }
}

async fn count(db: &Db, table: &str) -> Result<u64> {
    let sql = Sql::raw(db.dialect(), format!("SELECT COUNT(*) AS n FROM {table}"));
    Ok(db
        .fetch_int(&sql)
        .await?
        .map(|value| value.max(0) as u64)
        .unwrap_or(0))
}

/// 把时间文本解析成时间戳，供装配层校验入参。
pub fn parse_time(text: &str) -> Result<Timestamp> {
    Timestamp::parse(text).map_err(Error::from)
}
