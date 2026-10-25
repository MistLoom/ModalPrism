// 内容加工：工作流、模型、运行记录与通知接入。
//
// 三类声明（workflows / models）与运行记录分四个视图，共用同一份目录快照；
// 编辑与运行细节在 `components/workflow.tsx`，这里只负责列表、启停与试发通知。

import { useCallback, useEffect, useMemo, useState } from "react";

import { Field, SelectField, TextField } from "../components/forms";
import { IconAlert, IconFlow, IconPause, IconPlay, IconPlus, IconRefresh, IconTrash } from "../components/icons";
import { Card, Empty, Pill, SearchInput, Seg, Skeleton } from "../components/ui";
import { ModelDrawer, RunDrawer, WorkflowDrawer } from "../components/workflow";
import { useConsole } from "../lib/console";
import { relative, stamp } from "../lib/format";
import {
  NOTIFY_KINDS,
  catalog,
  modeLabel,
  modelTarget,
  notifyLabel,
  notifyTest,
  removeModel,
  runs as fetchRuns,
  statusLabel,
  stepKindLabel,
  tick,
  toggle,
  triggerLabel,
  type ModelRow,
  type NotifyKind,
  type WorkflowCatalog,
  type WorkflowRow,
  type WorkflowRun,
} from "../lib/workflows";

type Tab = "workflows" | "models" | "runs" | "notify";

const TABS: { value: Tab; label: string }[] = [
  { value: "workflows", label: "工作流" },
  { value: "models", label: "模型" },
  { value: "runs", label: "运行记录" },
  { value: "notify", label: "通知接入" },
];

export function WorkflowsPage() {
  const { notify, refresh } = useConsole();
  const [data, setData] = useState<WorkflowCatalog | null>(null);
  const [log, setLog] = useState<WorkflowRun[]>([]);
  const [failure, setFailure] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("workflows");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [ticking, setTicking] = useState(false);
  const [editor, setEditor] = useState<{ item: WorkflowRow | null } | null>(null);
  const [modelEditor, setModelEditor] = useState<{ item: ModelRow | null } | null>(null);
  const [viewing, setViewing] = useState<WorkflowRun | null>(null);

  const reload = useCallback(async () => {
    try {
      const [next, history] = await Promise.all([catalog(), fetchRuns(30).catch(() => [])]);
      setData(next);
      setLog(history);
      setFailure(null);
    } catch (problem) {
      setFailure(problem instanceof Error ? problem.message : "读取工作流失败");
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const workflows = data?.workflows ?? [];
  const models = data?.models ?? [];
  const usable = models.filter((model) => !model.error);
  const modelNames = useMemo(() => new Map(models.map((model) => [model.slug, model.name])), [models]);

  const filtered = useMemo(() => {
    const text = query.trim().toLowerCase();
    if (!text) {
      return workflows;
    }
    return workflows.filter(
      (item) =>
        item.name.toLowerCase().includes(text) ||
        item.slug.toLowerCase().includes(text) ||
        (item.description ?? "").toLowerCase().includes(text),
    );
  }, [workflows, query]);

  async function toggleRow(item: WorkflowRow) {
    setBusy(item.slug);
    try {
      await toggle(item.slug, !item.enabled);
      notify("ok", item.enabled ? `已停用 ${item.name}` : `已启用 ${item.name}`, "下一轮加工起生效。");
      await reload();
    } catch (problem) {
      notify("bad", "写入失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(null);
    }
  }

  async function dropModel(item: ModelRow) {
    if (!window.confirm(`删除模型 ${item.slug}？仍被工作流引用时会被校验拦下。`)) {
      return;
    }
    setBusy(item.slug);
    try {
      await removeModel(item.slug);
      notify("ok", `已删除模型 ${item.slug}`, "工作流若仍引用它，这次删除会被拦下并恢复原文。");
      await reload();
    } catch (problem) {
      notify("bad", "删除失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(null);
    }
  }

  async function tickNow() {
    setTicking(true);
    try {
      const result = await tick();
      notify(
        result.failed ? "warn" : "ok",
        `扫描 ${result.scanned ?? 0} 条，处理 ${result.processed ?? 0} 条`,
        result.failed ? `其中 ${result.failed} 条失败，详情见运行记录。` : "加工结果已写进内容分层。",
      );
      await reload();
      refresh();
      setTab("runs");
    } catch (problem) {
      notify("bad", "处理失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setTicking(false);
    }
  }

  const loading = data === null && failure === null;

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <span className="page-kicker">workflows · 内容加工</span>
          <h1 className="page-title">内容加工</h1>
          <p className="page-lede">
            新内容入库后按工作流跑模型、拼模板或调外部接口，结果写回内容分层；成功与失败都能推到飞书或自定义 Webhook。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={() => void reload()}>
            <IconRefresh size={13} /> 刷新
          </button>
          <button type="button" className="btn" disabled={ticking} onClick={() => void tickNow()}>
            <IconFlow size={13} /> {ticking ? "处理中…" : "处理新内容"}
          </button>
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => (tab === "models" ? setModelEditor({ item: null }) : setEditor({ item: null }))}
          >
            <IconPlus size={13} /> {tab === "models" ? "新建模型" : "新建工作流"}
          </button>
        </div>
      </header>

      <div className="toolbar">
        <Seg value={tab} onChange={setTab} options={TABS} label="内容加工视图" />
        {tab === "workflows" ? (
          <SearchInput value={query} onChange={setQuery} placeholder="按名称或说明筛选…" label="筛选工作流" />
        ) : null}
        <span className="spacer" />
        <span className="faint">
          {workflows.length} 个工作流 · {models.length} 个模型 · 最近 {log.length} 次运行
        </span>
      </div>

      {failure ? (
        <div className="note is-bad">
          <IconAlert size={14} />
          <span>
            <strong>读取失败</strong> · {failure}
          </span>
        </div>
      ) : null}

      {loading ? (
        <Card tight>
          <div style={{ padding: 18 }}>
            <Skeleton lines={8} />
          </div>
        </Card>
      ) : null}

      {!loading && tab === "workflows" ? (
        <>
          {models.length === 0 ? (
            <div className="note is-warn">
              <IconAlert size={14} />
              <span>
                <strong>还没有模型</strong> · 工作流必须引用一个模型。先到「模型」页建一个（远端 API、本地端点或命令行都可以），
                再回来建工作流。
                <button type="button" className="btn btn-sm" style={{ marginLeft: 10 }} onClick={() => setTab("models")}>
                  去建模型
                </button>
              </span>
            </div>
          ) : null}
          {filtered.length === 0 ? (
            <Card tight>
              <Empty
                title={workflows.length === 0 ? "还没有工作流" : "没有匹配的工作流"}
                text={
                  workflows.length === 0
                    ? "点右上角「新建工作流」，从要点摘要模板起步：一个模型步骤加一段产出模板就能跑。"
                    : "换个关键词试试。"
                }
              />
            </Card>
          ) : (
            <div className="wf-grid">
              {filtered.map((item) => (
                <article className={`wf-card${item.enabled ? "" : " is-dim"}`} key={item.slug}>
                  <header className="wf-card-head">
                    <div className="wf-card-text">
                      <span className="wf-card-name">{item.name}</span>
                      <span className="wf-card-desc">{item.description || "没有说明"}</span>
                    </div>
                    <button
                      type="button"
                      className="btn btn-sm btn-quiet"
                      disabled={busy === item.slug}
                      onClick={() => void toggleRow(item)}
                      aria-label={item.enabled ? "停用" : "启用"}
                    >
                      {item.enabled ? <IconPause size={12} /> : <IconPlay size={12} />}
                      {item.enabled ? "停用" : "启用"}
                    </button>
                  </header>

                  <div className="wf-flow" aria-label="加工流程">
                    <span className="wf-node is-model">
                      {item.model ? (modelNames.get(item.model) ?? item.model) : "未指定模型"}
                    </span>
                    {(item.steps ?? []).map((step) => (
                      <span className="wf-node" key={step.id}>
                        {stepKindLabel(step.kind)}
                        <b>{step.id}</b>
                      </span>
                    ))}
                    {item.outputs ? (
                      <span className="wf-node is-out">
                        产出 {item.outputs.kind}
                        {item.outputs.variant ? `/${item.outputs.variant}` : ""}
                      </span>
                    ) : null}
                  </div>

                  <div className="wf-card-meta">
                    <span className="tag">{triggerLabel(item.trigger?.on)}</span>
                    {item.trigger?.sources?.length ? (
                      <span className="tag">
                        限定 <strong>{item.trigger.sources.length}</strong> 个信源
                      </span>
                    ) : null}
                    {(item.notify ?? []).map((row, index) => (
                      <span className="tag" key={`${row.kind}-${index}`}>
                        {notifyLabel(row.kind)}
                      </span>
                    ))}
                    {item.available === false ? <Pill tone="warn">模型缺失</Pill> : null}
                    {item.error ? <Pill tone="bad">声明无效</Pill> : null}
                  </div>

                  {item.error ? (
                    <p className="note is-bad wf-card-error">
                      <IconAlert size={13} />
                      <span>{item.error}</span>
                    </p>
                  ) : null}

                  <footer className="wf-card-foot">
                    <span className="mono faint wf-card-slug">{item.file ?? `workflows/${item.slug}.yaml`}</span>
                    <span className="spacer" />
                    <button type="button" className="btn btn-sm" onClick={() => setEditor({ item })}>
                      编辑
                    </button>
                  </footer>
                </article>
              ))}
            </div>
          )}
        </>
      ) : null}

      {!loading && tab === "models" ? (
        <>
          <p className="faint wf-hint">
            三种调用方式：远端 API（OpenAI 兼容）、本地模型（Ollama / vLLM / LM Studio）与命令行（提示词进标准输入）。
            密钥只写环境变量名。
          </p>
          {models.length === 0 ? (
            <Card tight>
              <Empty
                title="还没有模型"
                text="先建一个模型，工作流才能引用它。远端 API 与本地端点都走同一套 chat completions。"
                action={
                  <button type="button" className="btn btn-primary btn-sm" onClick={() => setModelEditor({ item: null })}>
                    <IconPlus size={12} /> 新建模型
                  </button>
                }
              />
            </Card>
          ) : (
            <div className="wf-grid">
              {models.map((model) => (
                <article className={`wf-model${model.error ? " is-bad" : ""}`} key={model.slug}>
                  <header className="wf-card-head">
                    <div className="wf-card-text">
                      <span className="wf-card-name">{model.name}</span>
                      <span className="wf-card-desc mono">{modelTarget(model)}</span>
                    </div>
                    <Pill tone={model.error ? "bad" : "info"}>{modeLabel(model.mode)}</Pill>
                  </header>
                  <dl className="defs wf-model-defs">
                    <dt>声明</dt>
                    <dd className="mono">models/{model.slug}.yaml</dd>
                    {model.keyEnv ? (
                      <>
                        <dt>密钥变量</dt>
                        <dd className="mono">{model.keyEnv}</dd>
                      </>
                    ) : null}
                  </dl>
                  {model.error ? <p className="note is-bad wf-card-error">{model.error}</p> : null}
                  <footer className="wf-card-foot">
                    <span className="spacer" />
                    <button type="button" className="btn btn-sm" onClick={() => setModelEditor({ item: model })}>
                      编辑
                    </button>
                    <button
                      type="button"
                      className="btn btn-sm btn-quiet"
                      disabled={busy === model.slug}
                      onClick={() => void dropModel(model)}
                    >
                      <IconTrash size={12} /> 删除
                    </button>
                  </footer>
                </article>
              ))}
            </div>
          )}
        </>
      ) : null}

      {!loading && tab === "runs" ? (
        <Card
          title="最近运行"
          sub="运行记录写进内容分层（facet: struct / run），失败与成功都留档"
          tight
          action={
            <button type="button" className="btn btn-sm" onClick={() => void reload()}>
              <IconRefresh size={12} /> 刷新
            </button>
          }
        >
          {log.length === 0 ? (
            <Empty
              title="还没有运行记录"
              text={
                usable.length
                  ? "工作流会在新内容入库后自动运行；也可以点右上角「处理新内容」立刻跑一轮。"
                  : "先建模型与工作流，处理新内容时才会产生记录。"
              }
            />
          ) : (
            <table className="table">
              <thead>
                <tr>
                  <th>状态</th>
                  <th>工作流</th>
                  <th>内容</th>
                  <th>模型</th>
                  <th className="align-right">步骤</th>
                  <th className="align-right">耗时</th>
                  <th>时间</th>
                </tr>
              </thead>
              <tbody>
                {log.map((run) => (
                  <tr
                    key={run.runRid ?? `${run.workflow}-${run.rid}-${run.seq}`}
                    className="row-button"
                    onClick={() => setViewing(run)}
                  >
                    <td>
                      <Pill tone={run.status === "ok" ? "ok" : "bad"} dot>
                        {statusLabel(run.status)}
                      </Pill>
                    </td>
                    <td>
                      <div className="cell-main">
                        <span className="cell-strong">{run.name}</span>
                        <span className="cell-meta mono">{run.workflow}</span>
                      </div>
                    </td>
                    <td className="mono cell-meta">{shortRid(run.rid, 22)}</td>
                    <td className="mono cell-meta">{run.model}</td>
                    <td className="align-right">{run.steps.length}</td>
                    <td className="align-right mono cell-meta">{run.durationMs} ms</td>
                    <td className="cell-meta" title={stamp(run.recordedAt ?? run.startedAt)}>
                      {relative(run.recordedAt ?? run.startedAt)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Card>
      ) : null}

      {!loading && tab === "notify" ? <NotifyGuide /> : null}

      {editor ? (
        <WorkflowDrawer
          key={editor.item?.slug ?? "new-workflow"}
          item={editor.item}
          models={models}
          templates={data?.templates.workflows ?? []}
          onClose={() => setEditor(null)}
          onSaved={(message) => {
            notify("ok", message, "声明已写进 config/workflows，下一次加工即生效。");
            void reload();
          }}
        />
      ) : null}

      {modelEditor ? (
        <ModelDrawer
          key={modelEditor.item?.slug ?? "new-model"}
          item={modelEditor.item}
          templates={data?.templates.models ?? []}
          onClose={() => setModelEditor(null)}
          onSaved={(message) => {
            notify("ok", message, "声明已写进 config/models；已被工作流引用的模型不能删除。");
            void reload();
          }}
        />
      ) : null}

      {viewing ? <RunDrawer run={viewing} onClose={() => setViewing(null)} /> : null}

      <p className="faint wf-foot">
        工作流与模型写在 config/workflows 与 config/models；运行记录与产出写进内容分层。通知复用系统代理设置。
      </p>
    </div>
  );
}

// ── 通知接入 ────────────────────────────────────────────────────────────────

function NotifyGuide() {
  const { notify } = useConsole();
  const [kind, setKind] = useState<NotifyKind>("feishu");
  const [url, setUrl] = useState("");
  const [title, setTitle] = useState("ModalPrism 测试");
  const [sending, setSending] = useState(false);

  async function send() {
    if (!url.trim()) {
      notify("bad", "还缺地址", "粘贴飞书机器人或自定义 Webhook 的地址。");
      return;
    }
    setSending(true);
    try {
      await notifyTest({ kind, url: url.trim(), title: title.trim() || undefined });
      notify("ok", "测试通知已投递", "到对应群里看一眼；失败会返回平台错误码。");
    } catch (problem) {
      notify("bad", "投递失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setSending(false);
    }
  }

  return (
    <div className="wf-notify-grid">
      <Card title="试发一条" sub="用真实地址验证机器人与代理是否可用">
        <div className="form-grid">
          <Field label="通知方式" hint="飞书会用文本消息格式包一层">
            <SelectField
              value={kind}
              onChange={(next) => setKind(next as NotifyKind)}
              options={NOTIFY_KINDS}
            />
          </Field>
          <Field label="标题" hint="飞书关键词校验时，消息里必须含你设置的关键词">
            <TextField value={title} onChange={setTitle} placeholder="ModalPrism 测试" />
          </Field>
          <Field
            label="地址"
            hint={kind === "feishu" ? "形如 https://open.feishu.cn/open-apis/bot/v2/hook/…" : "任意接受 POST JSON 的地址"}
            wide
          >
            <TextField
              value={url}
              onChange={setUrl}
              placeholder={kind === "feishu" ? "https://open.feishu.cn/open-apis/bot/v2/hook/…" : "https://example.com/webhook"}
              mono
            />
          </Field>
        </div>
        <div className="row" style={{ marginTop: 12 }}>
          <button type="button" className="btn btn-primary btn-sm" disabled={sending} onClick={() => void send()}>
            {sending ? "投递中…" : "发送测试"}
          </button>
          <span className="faint">走系统代理设置；15 秒无响应即失败。</span>
        </div>
      </Card>

      <Card title="飞书机器人" sub="群机器人 Webhook，两步接好">
        <ol className="wf-guide">
          <li>
            <strong>拿到 Webhook</strong>
            <span>
              在飞书群里点右上角「…」→ 设置 → 群机器人 → 添加机器人 → 自定义机器人，复制
              <code className="mono">https://open.feishu.cn/open-apis/bot/v2/hook/…</code>。
            </span>
          </li>
          <li>
            <strong>设好安全策略</strong>
            <span>
              推荐「自定义关键词」，填 <code className="mono">ModalPrism</code>；通知标题默认就是它，也可以在通知里改成别的关键词。
              目前不支持签名校验（secret），请不要选「签名校验」。
            </span>
          </li>
          <li>
            <strong>填进工作流</strong>
            <span>
              编辑工作流 → 通知 → 添加一条，方式选「飞书机器人」，粘贴地址，勾选「失败时」（默认只推失败，成功也推会刷屏）。
            </span>
          </li>
        </ol>
        <p className="form-hint">
          飞书返回 HTTP 200 但 <code className="mono">code != 0</code> 也算失败，界面会带上平台错误码。
        </p>
      </Card>

      <Card title="通用 Webhook" sub="POST JSON，字段固定" >
        <pre className="code">{`POST <你的地址>
content-type: application/json

{
  "title": "文章标题",
  "workflow": "digest",
  "status": "ok",        // ok | failed
  "rid": "b3:…",
  "durationMs": 1840,
  "error": null          // 失败时是错误文本
}`}</pre>
        <p className="form-hint">
          通知投递失败只记日志，不回滚已经落盘的加工结果；需要重试时重新运行工作流或补发通知。
        </p>
      </Card>

      <Card title="分发预留" sub="同一份产出可以推给多个下游">
        <p className="muted">
          工作流声明里的 <code className="mono">notify</code> 是数组，同一次运行可以同时推飞书与自有 Webhook；
          产出本身写在内容分层的 facet 上，后续分发通道（邮件、RSS 输出、站点同步）只需读同一份 facet，不需要改工作流。
        </p>
        <div className="row" style={{ marginTop: 10 }}>
          <Pill tone="info">facet · 产出</Pill>
          <Pill tone="info">notify · 通知</Pill>
          <Pill tone="idle">下一步 · 分发通道</Pill>
        </div>
      </Card>
    </div>
  );
}

function shortRid(rid: string, width: number): string {
  if (rid.length <= width) {
    return rid;
  }
  return `${rid.slice(0, width - 1)}…`;
}
