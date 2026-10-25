//! 加工引擎。
//!
//! 一次 `tick` = 读游标 → 取游标之后的文章 → 逐条跑匹配的工作流 → 一次合并写入 → 投递通知。
//!
//! 三条约定：
//!
//! - 每次运行都留下一条 `struct:workflow-run` 记录，成功与失败都留，游标只在写入成功后前进；
//! - 产出写在文章资源上（`outputs.facet`），运行记录只描述过程，避免把过程当内容；
//! - 通知在写入成功之后才发，不会出现「通知说成功、存储里没有」。
//!
//! 加工链路的扩展点留在 `trigger`、`steps` 与 `outputs` 三处：新增触发条件、步骤种类或
//! 产出目标都只改 [`crate::spec`]，引擎按枚举分派，不写特例分支。

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egress::ProxySetting;
use serde_json::{Map, Value, json};
use strata_model::{
    ApplyOpts, Body, ChangeSet, Expr, FacetKind, FacetRef, FacetSelector, LineageInput, Op,
    OrderBy, Producer, Query, QueryFrom, Rid, SelectItem, SpanRef, Target, VersionSel,
};

use crate::error::{Error, Result};
use crate::http;
use crate::llm;
use crate::notify;
use crate::render;
use crate::spec::{ModelSpec, Notify, Step, StepKind, WorkflowSpec};
use crate::store::Store;

/// 每次 tick 最多取多少篇文章。加工要调模型，宁可多跑几轮，也不要一口气压满。
const PAGE: u32 = 20;
/// 运行记录里保留的步骤输出长度。
const STEP_CLIP: usize = 500;
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const STATE_SCOPE: &str = "workflow";
const STATE_KEY: &str = "commit-cursor";
const RUN_VARIANT: &str = "workflow-run";

/// 一次 tick 的结果。
#[derive(Debug, Default)]
pub struct Tick {
    pub scanned: usize,
    pub processed: usize,
    pub failed: usize,
    pub cursor: u64,
    pub next: u64,
    pub runs: Vec<Value>,
}

impl Tick {
    pub fn value(&self) -> Value {
        json!({
            "scanned": self.scanned,
            "processed": self.processed,
            "failed": self.failed,
            "cursor": self.cursor,
            "next": self.next,
            "runs": self.runs,
        })
    }
}

pub struct Engine {
    ctx: Arc<dyn types::ReadCtx>,
    store: Store,
}

impl Engine {
    pub fn new(ctx: Arc<dyn types::ReadCtx>) -> Self {
        let store = Store::new(ctx.config_dir());
        Self { ctx, store }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// 处理游标之后的新文章。没有新文章或没有启用的工作流时不做任何写入。
    pub async fn tick(&self) -> Result<Tick> {
        let (cursor, version) = self.cursor().await?;
        let specs = self.store.runnable()?;
        if specs.is_empty() {
            return Ok(Tick {
                cursor,
                next: cursor,
                ..Tick::default()
            });
        }
        let rows = self.pending(cursor).await?;
        let mut tick = Tick {
            cursor,
            next: cursor,
            scanned: rows.len(),
            ..Tick::default()
        };
        if rows.is_empty() {
            return Ok(tick);
        }
        let proxy = self.proxy();
        let mut ops = Vec::new();
        let mut pending = Vec::new();
        for row in &rows {
            tick.next = tick.next.max(row.seq);
            let Some(mut article) = self.article(&row.rid).await? else {
                continue;
            };
            article["seq"] = json!(row.seq);
            let vars = article_vars(&article);
            for (slug, spec) in &specs {
                if !matches(spec, &row.source_id) {
                    continue;
                }
                let outcome = self
                    .execute(slug, spec, &row.rid, row.seq, &vars, &proxy)
                    .await;
                if outcome.success {
                    tick.processed += 1;
                } else {
                    tick.failed += 1;
                }
                ops.extend(outcome.ops);
                pending.extend(outcome.pending);
                tick.runs.push(outcome.summary);
            }
        }
        if tick.next > cursor {
            ops.push(Op::SetState {
                scope: STATE_SCOPE.into(),
                key: STATE_KEY.into(),
                value: json!(tick.next),
                expect_version: (version > 0).then_some(version),
            });
        }
        if ops.is_empty() {
            return Ok(tick);
        }
        let snapshot = self.ctx.config().applied();
        self.ctx
            .strata()
            .apply(
                ChangeSet {
                    producer: Producer {
                        module: "workflow".into(),
                        version: "1".into(),
                        config_generation: snapshot.generation,
                    },
                    idempotency_key: None,
                    ack: None,
                    ops,
                },
                ApplyOpts::default(),
            )
            .await?;
        self.ctx.strata().flush().await?;
        self.deliver(pending, &proxy).await;
        Ok(tick)
    }

    /// 手动跑一条工作流。`trigger` 不参与，用来补跑或试跑。
    pub async fn run(&self, slug: &str, rid: &str) -> Result<Value> {
        let rid = Rid::parse(rid).map_err(|_| Error::invalid(format!("资源 id 无效：{rid}")))?;
        let spec = self.store.spec(slug)?;
        let Some(mut article) = self.article(&rid).await? else {
            return Err(Error::invalid(format!("文章不存在：{rid}")));
        };
        let seq = self.seq_of(&rid).await?.unwrap_or(0);
        article["seq"] = json!(seq);
        let proxy = self.proxy();
        let vars = article_vars(&article);
        let outcome = self.execute(slug, &spec, &rid, seq, &vars, &proxy).await;
        if !outcome.ops.is_empty() {
            let snapshot = self.ctx.config().applied();
            self.ctx
                .strata()
                .apply(
                    ChangeSet {
                        producer: Producer {
                            module: "workflow".into(),
                            version: "1".into(),
                            config_generation: snapshot.generation,
                        },
                        idempotency_key: None,
                        ack: None,
                        ops: outcome.ops,
                    },
                    ApplyOpts::default(),
                )
                .await?;
            self.ctx.strata().flush().await?;
        }
        self.deliver(outcome.pending, &proxy).await;
        Ok(outcome.summary)
    }

    /// 最近的运行记录。
    pub async fn runs(&self, limit: u32) -> Result<Value> {
        let page = self
            .ctx
            .strata()
            .query(Query {
                from: QueryFrom::Resources,
                r#where: None,
                has_facets: vec![selector(Some(RUN_VARIANT))],
                select: vec![
                    SelectItem::Field("seq".into()),
                    SelectItem::Field("createdAt".into()),
                    SelectItem::Facet(selector(Some(RUN_VARIANT))),
                ],
                expand: Vec::new(),
                search: None,
                as_of: None,
                order_by: vec![OrderBy {
                    field: "seq".into(),
                    descending: true,
                }],
                limit: Some(limit.clamp(1, 200)),
                cursor: None,
                include_tombstoned: false,
            })
            .await?;
        Ok(Value::Array(page.rows.iter().map(run_row).collect()))
    }

    /// 试投一条通知。界面用它验证机器人与代理是否可用。
    pub async fn notify_test(&self, input: &Value) -> Result<Value> {
        let notify = Notify {
            kind: serde_json::from_value(input.get("kind").cloned().unwrap_or(json!("webhook")))
                .map_err(|_| Error::invalid("通知方式只能是 webhook 或 feishu"))?,
            url: input
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            on: Vec::new(),
            title: input
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_string),
        };
        if !(notify.url.starts_with("http://") || notify.url.starts_with("https://")) {
            return Err(Error::invalid("通知地址必须是 HTTP 地址"));
        }
        let payload = json!({
            "workflow": "测试",
            "status": "ok",
            "title": "这是一条测试通知",
            "message": "收到说明通知链路可用",
        });
        notify::deliver(&notify, &payload, &self.proxy()).await?;
        Ok(json!({"ok": true}))
    }

    async fn execute(
        &self,
        slug: &str,
        spec: &WorkflowSpec,
        rid: &Rid,
        seq: u64,
        vars: &Value,
        proxy: &ProxySetting,
    ) -> Outcome {
        let started = Instant::now();
        let started_at = strata_model::Timestamp::now();
        let mut vars = vars.clone();
        let mut report: Vec<Value> = Vec::new();
        let mut error: Option<String> = None;
        for step in &spec.steps {
            let mark = Instant::now();
            match self.step(spec, step, &mut vars, proxy).await {
                Ok(output) => {
                    vars["steps"][step.id.as_str()] = json!(output);
                    report.push(json!({
                        "id": step.id,
                        "kind": step.kind.label(),
                        "status": "ok",
                        "ms": mark.elapsed().as_millis() as u64,
                        "output": http::clip(&output, STEP_CLIP),
                    }));
                }
                Err(failure) => {
                    report.push(json!({
                        "id": step.id,
                        "kind": step.kind.label(),
                        "status": "failed",
                        "ms": mark.elapsed().as_millis() as u64,
                        "error": failure.to_string(),
                    }));
                    error = Some(failure.to_string());
                    break;
                }
            }
        }
        let mut ops = Vec::new();
        if error.is_none() {
            match self.output_ops(spec, &mut vars, rid, slug) {
                Ok(list) => ops = list,
                Err(failure) => error = Some(failure.to_string()),
            }
        }
        let status = if error.is_some() { "failed" } else { "ok" };
        let ms = started.elapsed().as_millis() as u64;
        let title = vars
            .pointer("/article/title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let alias = format!("run-{slug}-{seq}");
        let mut all = vec![Op::Ensure {
            identity: format!("ext:workflow:{slug}:{rid}:{seq}"),
            ns: None,
            labels: BTreeMap::from([("workflow".to_string(), slug.to_string())]),
            r#as: alias.clone(),
        }];
        let record = json!({
            "workflow": slug,
            "name": spec.name.clone().unwrap_or_else(|| slug.to_string()),
            "status": status,
            "rid": rid.to_string(),
            "seq": seq,
            "model": spec.model,
            "startedAt": started_at,
            "durationMs": ms,
            "steps": report,
            "error": error,
            "output": spec.outputs.facet.as_ref().map(|facet| json!({
                "kind": facet.kind,
                "variant": facet.variant,
            })),
        });
        all.push(Op::PutFacet {
            target: Target::Alias(alias.clone()),
            kind: FacetKind::Struct,
            variant: Some(RUN_VARIANT.into()),
            body: Body::Struct {
                value: record.clone(),
            },
            media_type: Some("application/json".into()),
            lineage: LineageInput {
                inputs: self.inputs(rid).into_iter().collect(),
                params_hash: format!("workflow/{slug}"),
            },
        });
        all.extend(ops);
        all.push(Op::Link {
            src: Target::Alias(alias),
            rel: "generated_from".into(),
            dst: Target::Rid(rid.clone()),
            attrs: Some(json!({"workflow": slug, "status": status})),
        });
        let success = status == "ok";
        let payload = json!({
            "workflow": slug,
            "status": status,
            "title": title,
            "rid": rid.to_string(),
            "durationMs": ms,
            "error": record.get("error").cloned().unwrap_or(Value::Null),
        });
        let pending = spec
            .notify
            .iter()
            .filter(|item| item.wants(success))
            .cloned()
            .map(|item| (item, payload.clone()))
            .collect();
        Outcome {
            summary: record,
            ops: all,
            pending,
            success,
        }
    }

    async fn step(
        &self,
        spec: &WorkflowSpec,
        step: &Step,
        vars: &mut Value,
        proxy: &ProxySetting,
    ) -> Result<String> {
        match step.kind {
            StepKind::Llm => {
                let prompt = render::render(step.prompt.as_deref().unwrap_or_default(), vars)?;
                let model = self.model(spec, step.model.as_deref())?;
                llm::complete(&model, &prompt, proxy).await
            }
            StepKind::Template => {
                render::render(step.template.as_deref().unwrap_or_default(), vars)
            }
            StepKind::Http => {
                let url = render::render(step.url.as_deref().unwrap_or_default(), vars)?;
                let mut headers = Vec::with_capacity(step.headers.len());
                for (name, value) in &step.headers {
                    headers.push((name.clone(), render::render(value, vars)?));
                }
                let body = match &step.body {
                    Some(template) => Some(render::render(template, vars)?),
                    None => None,
                };
                let method = step.method.clone().unwrap_or_else(|| "POST".into());
                let reply =
                    http::request(&method, &url, &headers, body, HTTP_TIMEOUT, proxy).await?;
                reply.ok(&format!("步骤 {}", step.id))
            }
        }
    }

    fn model(&self, spec: &WorkflowSpec, step: Option<&str>) -> Result<ModelSpec> {
        self.store.model(step.unwrap_or(&spec.model))
    }

    fn output_ops(
        &self,
        spec: &WorkflowSpec,
        vars: &mut Value,
        rid: &Rid,
        slug: &str,
    ) -> Result<Vec<Op>> {
        let Some(facet) = &spec.outputs.facet else {
            return Ok(Vec::new());
        };
        let body = render::render(&facet.body, vars)?;
        let (kind, variant, media_type, body) = match facet.kind.as_str() {
            "struct" => (
                FacetKind::Struct,
                Some(facet.variant.clone().unwrap_or_else(|| "workflow".into())),
                Some("application/json".to_string()),
                Body::Struct {
                    value: as_object(&body),
                },
            ),
            "md" => (
                FacetKind::Md,
                facet.variant.clone(),
                Some("text/markdown".to_string()),
                Body::Text { text: body },
            ),
            _ => (
                FacetKind::Text,
                facet.variant.clone(),
                Some("text/plain".to_string()),
                Body::Text { text: body },
            ),
        };
        Ok(vec![Op::PutFacet {
            target: Target::Rid(rid.clone()),
            kind,
            variant,
            body,
            media_type,
            lineage: LineageInput {
                inputs: self.inputs(rid).into_iter().collect(),
                params_hash: format!("workflow/{slug}/{}", facet.kind),
            },
        }])
    }

    fn inputs(&self, rid: &Rid) -> Option<SpanRef> {
        let reference = FacetRef::new(
            rid.clone(),
            FacetKind::Struct,
            Some("article".into()),
            VersionSel::Latest,
        )
        .ok()?;
        SpanRef::new(reference, 0, 0).ok()
    }

    async fn deliver(&self, pending: Vec<(Notify, Value)>, proxy: &ProxySetting) {
        for (notify, payload) in pending {
            if let Err(error) = notify::deliver(&notify, &payload, proxy).await {
                tracing::warn!(url = %notify.url, error = %error, "通知投递失败");
            }
        }
    }

    fn proxy(&self) -> ProxySetting {
        egress::setting_of(&self.ctx.config().applied().system)
    }

    async fn cursor(&self) -> Result<(u64, u64)> {
        let state = self.ctx.strata().state(STATE_SCOPE, STATE_KEY).await?;
        Ok(match state {
            Some(state) => (state.value.as_u64().unwrap_or(0), state.version),
            None => (0, 0),
        })
    }

    async fn pending(&self, cursor: u64) -> Result<Vec<Row>> {
        let filter = Expr::try_from(json!({"gt": ["seq", cursor]}))
            .map_err(|_| Error::invalid("游标表达式无效"))?;
        let page = self
            .ctx
            .strata()
            .query(Query {
                from: QueryFrom::Resources,
                r#where: Some(filter),
                has_facets: vec![selector(Some("article"))],
                select: Vec::new(),
                expand: Vec::new(),
                search: None,
                as_of: None,
                order_by: Vec::new(),
                limit: Some(PAGE),
                cursor: None,
                include_tombstoned: false,
            })
            .await?;
        let mut rows = Vec::new();
        for value in page.rows {
            let Some(rid) = value
                .get("rid")
                .and_then(Value::as_str)
                .and_then(|text| Rid::parse(text).ok())
            else {
                continue;
            };
            rows.push(Row {
                rid,
                seq: value.get("seq").and_then(Value::as_u64).unwrap_or(0),
                source_id: value
                    .get("sourceId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            });
        }
        Ok(rows)
    }

    async fn article(&self, rid: &Rid) -> Result<Option<Value>> {
        let value = types::Article::project(self.ctx.as_ref(), rid)
            .await
            .map_err(|error| Error::invalid(error.to_string()))?;
        if value.is_null() {
            return Ok(None);
        }
        Ok(Some(value))
    }

    async fn seq_of(&self, rid: &Rid) -> Result<Option<u64>> {
        let filter = Expr::try_from(json!({"eq": ["rid", rid.to_string()]}))
            .map_err(|_| Error::invalid("游标表达式无效"))?;
        let page = self
            .ctx
            .strata()
            .query(Query {
                from: QueryFrom::Resources,
                r#where: Some(filter),
                has_facets: Vec::new(),
                select: vec![SelectItem::Field("seq".into())],
                expand: Vec::new(),
                search: None,
                as_of: None,
                order_by: Vec::new(),
                limit: Some(1),
                cursor: None,
                include_tombstoned: false,
            })
            .await?;
        Ok(page
            .rows
            .first()
            .and_then(|row| row.get("seq"))
            .and_then(Value::as_u64))
    }
}

struct Row {
    rid: Rid,
    seq: u64,
    source_id: String,
}

struct Outcome {
    summary: Value,
    ops: Vec<Op>,
    pending: Vec<(Notify, Value)>,
    success: bool,
}

fn selector(variant: Option<&str>) -> FacetSelector {
    FacetSelector {
        kind: FacetKind::Struct,
        variant: variant.map(str::to_string),
        fields: Vec::new(),
    }
}

/// 触发条件：`sources` 为空表示全部；否则接受 `source/<slug>` 或 `<slug>`。
fn matches(spec: &WorkflowSpec, source_id: &str) -> bool {
    if !spec.trigger.kinds.iter().any(|kind| kind == "article") {
        return false;
    }
    if spec.trigger.sources.is_empty() {
        return true;
    }
    let slug = source_id.strip_prefix("source/").unwrap_or(source_id);
    spec.trigger
        .sources
        .iter()
        .any(|item| item == source_id || item == slug)
}

fn article_vars(article: &Value) -> Value {
    let mut object = article
        .get("struct")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    object.insert(
        "rid".into(),
        article.get("rid").cloned().unwrap_or(Value::Null),
    );
    let text = article
        .get("readable")
        .and_then(Value::as_str)
        .unwrap_or_default();
    object.insert("text".into(), json!(text));
    if let Some(seq) = article.get("seq") {
        object.insert("seq".into(), seq.clone());
    }
    json!({
        "article": Value::Object(object),
        "steps": Value::Object(Map::new()),
    })
}

fn as_object(text: &str) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(value @ Value::Object(_)) => value,
        _ => json!({"text": text}),
    }
}

fn run_row(row: &Value) -> Value {
    let key = format!("struct:{RUN_VARIANT}");
    let mut out = row
        .get(&key)
        .and_then(|facet| facet.get("value"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    out.insert(
        "runRid".into(),
        row.get("rid").cloned().unwrap_or(Value::Null),
    );
    if let Some(seq) = row.get("seq") {
        out.insert("seq".into(), seq.clone());
    }
    if let Some(created) = row.get("createdAt") {
        out.insert("recordedAt".into(), created.clone());
    }
    Value::Object(out)
}
