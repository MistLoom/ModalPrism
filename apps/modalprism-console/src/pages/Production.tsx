import { useCallback, useEffect, useState } from "react";

import { IconCheck, IconClose, IconPlus, IconRefresh } from "../components/icons";
import { Card, Drawer, Empty, Field, Loading, Modal, Problem, StatusBadge, Tabs, useUi } from "../components/ui";
import {
  createDraft,
  createInputSet,
  createProductionRun,
  createPublication,
  createWorkflow,
  createWorkflowVersion,
  getProductionRun,
  listCandidates,
  listChannels,
  listDrafts,
  listInputSets,
  listProductionRuns,
  listRunAttempts,
  listWorkflowVersions,
  listWorkflows,
  reviewCandidate,
  retryProductionRun,
  updateDraft,
  uploadInputFile,
} from "../lib/api";
import { relative, stamp } from "../lib/format";
import { ProjectScopeBadge } from "../lib/project";
import { useRoute } from "../lib/router";
import type {
  Attempt,
  Candidate,
  Channel,
  Draft,
  InputSet,
  ProductionRun,
  Workflow,
  WorkflowStep,
  WorkflowVersion,
} from "../lib/types";

type Tab = "inputs" | "workflows" | "runs" | "candidates" | "drafts";

export function ProductionPage() {
  const { params } = useRoute();
  const { notify, run } = useUi();
  const [tab, setTab] = useState<Tab>((params.get("tab") as Tab) || "inputs");
  const [inputs, setInputs] = useState<InputSet[]>([]);
  const [workflows, setWorkflows] = useState<Workflow[]>([]);
  const [versions, setVersions] = useState<WorkflowVersion[]>([]);
  const [selectedWorkflow, setSelectedWorkflow] = useState<string>("");
  const [runs, setRuns] = useState<ProductionRun[]>([]);
  const [candidates, setCandidates] = useState<Candidate[]>([]);
  const [statusFilter, setStatusFilter] = useState(params.get("status") ?? "all");
  const [drafts, setDrafts] = useState<Draft[]>([]);
  const [channels, setChannels] = useState<Channel[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [modal, setModal] = useState<null | "input" | "workflow" | "version" | "run" | "draft" | "publish">(null);
  const [runDetail, setRunDetail] = useState<ProductionRun | null>(null);
  const [candidateDetail, setCandidateDetail] = useState<Candidate | null>(null);
  const [draftDetail, setDraftDetail] = useState<Draft | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [inputPage, workflowPage, runPage, candidatePage, draftPage, channelPage] = await Promise.all([
        listInputSets({ limit: 100 }),
        listWorkflows({ limit: 100 }),
        listProductionRuns({ limit: 100, status: statusFilter === "all" ? undefined : statusFilter }),
        listCandidates({ limit: 100, status: statusFilter === "all" ? undefined : statusFilter }),
        listDrafts({ limit: 100 }),
        listChannels(),
      ]);
      setInputs(inputPage.items);
      setWorkflows(workflowPage.items);
      setRuns(runPage.items);
      setCandidates(candidatePage.items);
      setDrafts(draftPage.items);
      setChannels(channelPage.items);
      if (!selectedWorkflow && workflowPage.items.length) {
        setSelectedWorkflow(workflowPage.items[0].id);
      }
      setError(null);
    } catch (problem) {
      setError(problem);
    } finally {
      setLoading(false);
    }
  }, [statusFilter, selectedWorkflow]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!selectedWorkflow) {
      setVersions([]);
      return;
    }
    let alive = true;
    void listWorkflowVersions(selectedWorkflow)
      .then((page) => {
        if (alive) setVersions(page.items);
      })
      .catch(() => {
        if (alive) setVersions([]);
      });
    return () => {
      alive = false;
    };
  }, [selectedWorkflow]);

  const approved = candidates.filter((candidate) => candidate.status === "approved");

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <div className="kicker">production · 生产</div>
          <h1 className="page-title">从冻结输入到可审核候选</h1>
          <p className="page-lede">
            输入、工作流版本、运行与候选都是不可变记录。生成永远先成为候选，人工审核后才进入草稿与发布准备；自动与手动入口共用同一执行核心。
          </p>
        </div>
        <div className="page-actions">
          <ProjectScopeBadge />
          <button type="button" className="btn" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={14} /> 刷新
          </button>
          <button type="button" className="btn btn-primary" onClick={() => setModal("input")}>
            <IconPlus size={14} /> 新建输入集
          </button>
        </div>
      </header>

      {error ? <Problem error={error} onRetry={() => void load()} /> : null}

      <Tabs
        value={tab}
        onChange={setTab}
        items={[
          { id: "inputs", label: "输入集", count: inputs.length },
          { id: "workflows", label: "工作流", count: workflows.length },
          { id: "runs", label: "运行", count: runs.length },
          { id: "candidates", label: "候选", count: candidates.length },
          { id: "drafts", label: "草稿", count: drafts.length },
        ]}
      />

      {tab !== "inputs" && tab !== "workflows" && tab !== "drafts" ? (
        <div className="toolbar-band">
          <div className="seg" role="group" aria-label="状态筛选">
            {["all", "queued", "running", "awaitingReview", "succeeded", "failed"].map((status) => (
              <button key={status} type="button" className={statusFilter === status ? "is-active" : ""} onClick={() => setStatusFilter(status)}>
                {status === "all" ? "全部" : status}
              </button>
            ))}
          </div>
        </div>
      ) : null}

      {tab === "inputs" ? (
        <Card title="冻结输入" sub="创建后不可修改；文件与 URL 在创建时冻结" flush action={<button type="button" className="btn btn-sm" onClick={() => setModal("input")}><IconPlus size={12} /> 新建</button>}>
          {loading && inputs.length === 0 ? <div className="card-body"><Loading lines={4} /></div> : inputs.length === 0 ? (
            <Empty title="还没有输入集" text="支持内容、多项内容、查询快照、文本、文件、URL 与结构化数据。" action={<button type="button" className="btn btn-primary btn-sm" onClick={() => setModal("input")}>新建输入集</button>} />
          ) : (
            <div className="list">
              {inputs.map((input) => (
                <div className="list-row" key={input.id}>
                  <span className="list-main">
                    <span className="list-title">{input.name}</span>
                    <span className="list-meta">
                      <span className="mono">{input.id}</span>
                      <span>{input.inputs.length} 个输入</span>
                      <span>{relative(input.createdAt)}</span>
                    </span>
                  </span>
                  <button type="button" className="btn btn-sm" onClick={() => { setModal("run"); }}>
                    用它运行
                  </button>
                </div>
              ))}
            </div>
          )}
        </Card>
      ) : null}

      {tab === "workflows" ? (
        <div className="split-2">
          <Card title="工作流" sub="可编辑身份；版本是执行契约" flush action={<button type="button" className="btn btn-sm" onClick={() => setModal("workflow")}><IconPlus size={12} /> 新建</button>}>
            {workflows.length === 0 ? <Empty title="还没有工作流" text="创建一个模板直出或模型生成流程。" /> : (
              <div className="list">
                {workflows.map((workflow) => (
                  <button
                    type="button"
                    key={workflow.id}
                    className="list-row clickable"
                    style={{ background: selectedWorkflow === workflow.id ? "var(--accent-soft)" : "transparent", border: 0, borderBottom: "1px solid var(--line-soft)", textAlign: "left", width: "100%" }}
                    onClick={() => setSelectedWorkflow(workflow.id)}
                  >
                    <span className="list-main">
                      <span className="list-title">{workflow.name}</span>
                      <span className="list-meta">
                        <span>{workflow.description}</span>
                        <span>{workflow.versionCount} 个版本</span>
                      </span>
                    </span>
                    <span className="mono small">{workflow.latestVersionId ? workflow.latestVersionId.slice(-6) : "—"}</span>
                  </button>
                ))}
              </div>
            )}
          </Card>
          <Card title="版本" sub={selectedWorkflow || "选择左侧工作流"} flush action={<button type="button" className="btn btn-sm" disabled={!selectedWorkflow} onClick={() => setModal("version")}><IconPlus size={12} /> 新版本</button>}>
            {versions.length === 0 ? <Empty title="还没有版本" text="版本固定步骤、模型、提示词与依赖；发布后不可改。" /> : (
              <div className="list">
                {versions.map((version) => (
                  <div className="list-row" key={version.id}>
                    <span className="list-main">
                      <span className="list-title">v{version.version} · {version.steps.length} 步</span>
                      <span className="list-meta">
                        <span className="mono">{version.id}</span>
                        <span>{stamp(version.createdAt)}</span>
                      </span>
                    </span>
                    <button type="button" className="btn btn-sm" onClick={() => { setModal("run"); }}>运行</button>
                  </div>
                ))}
              </div>
            )}
          </Card>
        </div>
      ) : null}

      {tab === "runs" ? (
        <Card title="生产运行" sub="Attempt 重试保留历史" flush>
          {runs.length === 0 ? <Empty title="还没有运行" text="选择输入集与工作流版本创建一次运行。" action={<button type="button" className="btn btn-primary btn-sm" onClick={() => setModal("run")}>新建运行</button>} /> : (
            <div className="table-wrap">
              <table className="table">
                <thead><tr><th>运行</th><th>状态</th><th>模式</th><th>候选</th><th>创建</th><th /></tr></thead>
                <tbody>
                  {runs.map((item) => (
                    <tr key={item.id}>
                      <td><div className="cell-main"><span className="source-name mono">{item.id}</span><span className="source-sub">{item.inputSetId} · {item.workflowVersionId}</span></div></td>
                      <td><StatusBadge status={item.status} /></td>
                      <td className="small">{item.mode}</td>
                      <td className="mono">{item.candidateIds.length}</td>
                      <td className="small muted">{relative(item.createdAt)}</td>
                      <td className="actions">
                        <button type="button" className="btn btn-sm" onClick={() => setRunDetail(item)}>详情</button>
                        <button type="button" className="btn btn-sm" onClick={() => void (async () => {
                          let started = false;
                          try {
                            const operation = await retryProductionRun(item.id, "控制台重试");
                            started = true;
                            await run(operation, "重试生产运行");
                            await load();
                          } catch (problem) {
                            if (!started) {
                              notify("bad", "重试失败", problem instanceof Error ? problem.message : undefined);
                            }
                          }
                        })()}>重试</button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </Card>
      ) : null}

      {tab === "candidates" ? (
        candidates.length === 0 ? <Card title="候选"><Empty title="还没有候选" text="运行一个工作流后，生成结果会进入这里等待审核。" /></Card> : (
          <div className="candidate-grid">
            {candidates.map((candidate) => (
              <article className="candidate-card" key={candidate.id}>
                <div className="row between">
                  <StatusBadge status={candidate.status} />
                  <span className="small muted">{relative(candidate.createdAt)}</span>
                </div>
                <strong>{candidate.title}</strong>
                <div className="candidate-body">{candidate.body}</div>
                {candidate.validation.issues.length ? <div className="small" style={{ color: "var(--danger)" }}>{candidate.validation.issues.map((issue) => issue.message).join("；")}</div> : null}
                <div className="row">
                  <button type="button" className="btn btn-sm" onClick={() => setCandidateDetail(candidate)}>查看与审核</button>
                </div>
              </article>
            ))}
          </div>
        )
      ) : null}

      {tab === "drafts" ? (
        <Card title="草稿" sub="审核通过后进入编辑与发布准备" flush action={<button type="button" className="btn btn-sm" disabled={approved.length === 0} onClick={() => setModal("draft")}><IconPlus size={12} /> 由已批准候选创建</button>}>
          {drafts.length === 0 ? <Empty title="还没有草稿" text="先批准一个候选，再创建草稿。" /> : (
            <div className="list">
              {drafts.map((draft) => (
                <div className="list-row" key={draft.id}>
                  <span className="list-main">
                    <span className="list-title">{draft.title}</span>
                    <span className="list-meta">
                      <span className="mono">{draft.id}</span>
                      <span>v{draft.latestVersion}</span>
                      <span>{relative(draft.updatedAt)}</span>
                    </span>
                  </span>
                  <button type="button" className="btn btn-sm" onClick={() => setDraftDetail(draft)}>编辑</button>
                  <button type="button" className="btn btn-sm btn-primary" onClick={() => { setDraftDetail(draft); setModal("publish"); }}>发布</button>
                </div>
              ))}
            </div>
          )}
        </Card>
      ) : null}

      {modal === "input" ? <InputModal onClose={() => setModal(null)} onDone={() => { setModal(null); void load(); }} /> : null}
      {modal === "workflow" ? <WorkflowModal onClose={() => setModal(null)} onDone={() => { setModal(null); void load(); }} /> : null}
      {modal === "version" ? <VersionModal workflowId={selectedWorkflow} workflow={workflows.find((item) => item.id === selectedWorkflow)} onClose={() => setModal(null)} onDone={() => { setModal(null); void load(); }} /> : null}
      {modal === "run" ? <RunModal inputs={inputs} workflows={workflows} onClose={() => setModal(null)} onDone={() => { setModal(null); void load(); }} /> : null}
      {modal === "draft" ? <DraftModal candidates={approved} onClose={() => setModal(null)} onDone={() => { setModal(null); void load(); }} /> : null}
      {modal === "publish" && draftDetail ? <PublishModal draft={draftDetail} channels={channels} onClose={() => setModal(null)} onDone={() => { setModal(null); setDraftDetail(null); void load(); }} /> : null}
      {runDetail ? <RunDrawer run={runDetail} onClose={() => setRunDetail(null)} /> : null}
      {candidateDetail ? <CandidateDrawer candidate={candidateDetail} onClose={() => setCandidateDetail(null)} onDone={() => { setCandidateDetail(null); void load(); }} /> : null}
      {draftDetail && modal !== "publish" ? <DraftDrawer draft={draftDetail} onClose={() => setDraftDetail(null)} onDone={() => { setDraftDetail(null); void load(); }} /> : null}
    </div>
  );
}

function InputModal({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const { notify } = useUi();
  const [name, setName] = useState("");
  const [kind, setKind] = useState("text");
  const [text, setText] = useState("");
  const [contentIds, setContentIds] = useState("");
  const [url, setUrl] = useState("");
  const [structured, setStructured] = useState("{}");
  const [file, setFile] = useState<File | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function create() {
    setBusy(true);
    setError(null);
    try {
      let input: Record<string, unknown>;
      if (kind === "text") input = { kind, text };
      else if (kind === "content") input = { kind, contentId: contentIds.trim() };
      else if (kind === "contentList") input = { kind, contentIds: contentIds.split(/[\s,]+/).filter(Boolean) };
      else if (kind === "url") input = { kind, url };
      else if (kind === "structured") input = { kind, value: JSON.parse(structured) };
      else if (kind === "file") {
        if (!file) throw new Error("请选择文件");
        const uploaded = await uploadInputFile(file);
        input = { kind, fileId: uploaded.id };
      } else {
        throw new Error("不支持的输入类型");
      }
      await createInputSet({ name, inputs: [input] });
      notify("ok", "输入集已冻结");
      onDone();
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : "创建失败");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="新建输入集"
      sub="一个输入集可以混合多类输入；这里先添加一项"
      onClose={onClose}
      footer={<><button type="button" className="btn" onClick={onClose}>取消</button><button type="button" className="btn btn-primary" onClick={() => void create()} disabled={busy || !name.trim()}>{busy ? "创建中…" : "冻结创建"}</button></>}
    >
      <div className="stack" style={{ gap: 12 }}>
        {error ? <div className="alert bad">{error}</div> : null}
        <Field label="名称"><input className="input" value={name} onChange={(event) => setName(event.target.value)} /></Field>
        <Field label="输入类型">
          <select className="select" value={kind} onChange={(event) => setKind(event.target.value)}>
            <option value="text">文本</option>
            <option value="content">单条内容</option>
            <option value="contentList">多项内容</option>
            <option value="url">URL</option>
            <option value="structured">结构化数据</option>
            <option value="file">文件</option>
          </select>
        </Field>
        {kind === "text" ? <Field label="文本"><textarea className="textarea" value={text} onChange={(event) => setText(event.target.value)} /></Field> : null}
        {kind === "content" ? <Field label="内容 ID"><input className="input mono" value={contentIds} onChange={(event) => setContentIds(event.target.value)} placeholder="r_…" /></Field> : null}
        {kind === "contentList" ? <Field label="内容 ID" hint="用逗号或换行分隔"><textarea className="textarea" value={contentIds} onChange={(event) => setContentIds(event.target.value)} /></Field> : null}
        {kind === "url" ? <Field label="URL" hint="创建时抓取并冻结；运行期间不再读取浮动地址"><input className="input" value={url} onChange={(event) => setUrl(event.target.value)} /></Field> : null}
        {kind === "structured" ? <Field label="JSON"><textarea className="textarea" value={structured} onChange={(event) => setStructured(event.target.value)} /></Field> : null}
        {kind === "file" ? <Field label="文件"><input className="input" type="file" onChange={(event) => setFile(event.target.files?.[0] ?? null)} /></Field> : null}
      </div>
    </Modal>
  );
}

function WorkflowModal({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const { notify } = useUi();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <Modal title="新建工作流" onClose={onClose} footer={<><button type="button" className="btn" onClick={onClose}>取消</button><button type="button" className="btn btn-primary" disabled={busy || !name.trim()} onClick={() => void (async () => {
      setBusy(true);
      try {
        await createWorkflow({ name, description });
        notify("ok", "工作流已创建");
        onDone();
      } catch (problem) {
        notify("bad", "创建失败", problem instanceof Error ? problem.message : undefined);
      } finally {
        setBusy(false);
      }
    })()}>{busy ? "创建中…" : "创建"}</button></>}>
      <div className="stack" style={{ gap: 12 }}>
        <Field label="名称"><input className="input" value={name} onChange={(event) => setName(event.target.value)} /></Field>
        <Field label="说明"><textarea className="textarea" value={description} onChange={(event) => setDescription(event.target.value)} /></Field>
      </div>
    </Modal>
  );
}

function VersionModal({ workflowId, workflow, onClose, onDone }: { workflowId: string; workflow?: Workflow; onClose: () => void; onDone: () => void }) {
  const { notify } = useUi();
  const [steps, setSteps] = useState<WorkflowStep[]>([{ id: "直出", kind: "template", parameters: { template: "{{input.text}}" } }]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  function update(index: number, patch: Partial<WorkflowStep>) {
    setSteps((current) => current.map((step, at) => (at === index ? { ...step, ...patch } : step)));
  }

  return (
    <Modal
      title={`发布新版本 · ${workflow?.name ?? workflowId}`}
      sub="版本发布后不可修改；模板变量 {{input.text}} 与 {{steps.<id>}} 由执行核心渲染"
      onClose={onClose}
      wide
      footer={<><button type="button" className="btn" onClick={onClose}>取消</button><button type="button" className="btn btn-primary" disabled={busy || !workflow} onClick={() => void (async () => {
        setBusy(true);
        setError(null);
        try {
          await createWorkflowVersion(workflowId, steps, workflow?.etag ?? "");
          notify("ok", "工作流版本已发布");
          onDone();
        } catch (problem) {
          setError(problem instanceof Error ? problem.message : "发布失败");
        } finally {
          setBusy(false);
        }
      })()}>{busy ? "发布中…" : "发布版本"}</button></>}
    >
      <div className="stack" style={{ gap: 12 }}>
        {error ? <div className="alert bad">{error}</div> : null}
        {steps.map((step, index) => (
          <div className="step" key={index}>
            <span className="step-index">{index + 1}</span>
            <div className="stack" style={{ gap: 8 }}>
              <div className="grid cols-2">
                <Field label="步骤 ID"><input className="input mono" value={step.id} onChange={(event) => update(index, { id: event.target.value })} /></Field>
                <Field label="类型">
                  <select className="select" value={step.kind} onChange={(event) => update(index, { kind: event.target.value as WorkflowStep["kind"] })}>
                    <option value="template">模板</option>
                    <option value="model">模型</option>
                    <option value="externalCall">外部调用</option>
                    <option value="validation">校验</option>
                  </select>
                </Field>
              </div>
              {step.kind === "model" ? <Field label="模型 ID" hint="留空使用系统设置里的默认模型"><input className="input mono" value={step.modelId ?? ""} onChange={(event) => update(index, { modelId: event.target.value })} /></Field> : null}
              <Field label="参数 JSON" hint={step.kind === "template" ? '例如 {"template":"{{input.text}}"}' : step.kind === "validation" ? '例如 {"schema":{"type":"object"}}' : '例如 {"prompt":"用三句话概括 {{input.text}}"} 或 {"url":"https://..."}'}>
                <StepParameters step={step} onChange={(parameters) => update(index, { parameters })} />
              </Field>
            </div>
            <button type="button" className="icon-btn" onClick={() => setSteps((current) => current.filter((_, at) => at !== index))} aria-label="删除步骤"><IconClose size={15} /></button>
          </div>
        ))}
        <button type="button" className="btn btn-sm" onClick={() => setSteps((current) => [...current, { id: `步骤${current.length + 1}`, kind: "template", parameters: { template: "{{steps." + current[0]?.id + "}}" } }])}>
          <IconPlus size={12} /> 添加步骤
        </button>
      </div>
    </Modal>
  );
}

function StepParameters({ step, onChange }: { step: WorkflowStep; onChange: (value: Record<string, unknown>) => void }) {
  const [raw, setRaw] = useState(() => JSON.stringify(step.parameters ?? {}, null, 2));
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setRaw(JSON.stringify(step.parameters ?? {}, null, 2));
    setError(null);
  }, [step.id, step.kind]);
  return (
    <div className="stack" style={{ gap: 4 }}>
      <textarea
        className="textarea"
        value={raw}
        onChange={(event) => {
          const text = event.target.value;
          setRaw(text);
          try {
            const parsed = JSON.parse(text) as unknown;
            if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
              setError("参数必须是 JSON 对象");
              return;
            }
            setError(null);
            onChange(parsed as Record<string, unknown>);
          } catch {
            setError("JSON 尚未闭合或语法无效");
          }
        }}
      />
      {error ? <span className="field-error">{error}</span> : null}
    </div>
  );
}

function RunModal({ inputs, workflows, onClose, onDone }: { inputs: InputSet[]; workflows: Workflow[]; onClose: () => void; onDone: () => void }) {
  const { notify, run } = useUi();
  const [inputSetId, setInputSetId] = useState(inputs[0]?.id ?? "");
  const [workflowId, setWorkflowId] = useState(workflows[0]?.id ?? "");
  const [versions, setVersions] = useState<WorkflowVersion[]>([]);
  const [versionId, setVersionId] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!workflowId) return;
    let alive = true;
    void listWorkflowVersions(workflowId).then((page) => {
      if (!alive) return;
      setVersions(page.items);
      setVersionId(page.items[0]?.id ?? "");
    });
    return () => {
      alive = false;
    };
  }, [workflowId]);

  return (
    <Modal
      title="新建生产运行"
      sub="运行绑定一个冻结输入集与一个不可变工作流版本"
      onClose={onClose}
      footer={<><button type="button" className="btn" onClick={onClose}>取消</button><button type="button" className="btn btn-primary" disabled={busy || !inputSetId || !versionId} onClick={() => void (async () => {
        setBusy(true);
        let started = false;
        try {
          const operation = await createProductionRun({ inputSetId, workflowVersionId: versionId, mode: "manual" });
          started = true;
          await run(operation, "生产运行");
          onDone();
        } catch (problem) {
          if (!started) {
            notify("bad", "创建运行失败", problem instanceof Error ? problem.message : undefined);
          }
        } finally {
          setBusy(false);
        }
      })()}>{busy ? "运行中…" : "开始运行"}</button></>}
    >
      <div className="stack" style={{ gap: 12 }}>
        <Field label="输入集">
          <select className="select" value={inputSetId} onChange={(event) => setInputSetId(event.target.value)}>
            {inputs.map((input) => <option key={input.id} value={input.id}>{input.name} · {input.inputs.length} 项</option>)}
          </select>
        </Field>
        <Field label="工作流">
          <select className="select" value={workflowId} onChange={(event) => setWorkflowId(event.target.value)}>
            {workflows.map((workflow) => <option key={workflow.id} value={workflow.id}>{workflow.name}</option>)}
          </select>
        </Field>
        <Field label="版本">
          <select className="select" value={versionId} onChange={(event) => setVersionId(event.target.value)}>
            {versions.length === 0 ? <option value="">没有已发布版本</option> : null}
            {versions.map((version) => <option key={version.id} value={version.id}>v{version.version} · {version.steps.length} 步</option>)}
          </select>
        </Field>
      </div>
    </Modal>
  );
}

function DraftModal({ candidates, onClose, onDone }: { candidates: Candidate[]; onClose: () => void; onDone: () => void }) {
  const { notify } = useUi();
  const [candidateId, setCandidateId] = useState(candidates[0]?.id ?? "");
  const [title, setTitle] = useState(candidates[0]?.title ?? "");
  const [body, setBody] = useState(candidates[0]?.body ?? "");
  const [busy, setBusy] = useState(false);
  const selected = candidates.find((item) => item.id === candidateId);

  useEffect(() => {
    if (selected) {
      setTitle(selected.title);
      setBody(selected.body);
    }
  }, [selected]);

  return (
    <Modal title="创建草稿" sub="草稿来自已批准候选；修改会新增版本" onClose={onClose} footer={<><button type="button" className="btn" onClick={onClose}>取消</button><button type="button" className="btn btn-primary" disabled={busy || !candidateId || !title.trim()} onClick={() => void (async () => {
      setBusy(true);
      try {
        await createDraft({ candidateIds: [candidateId], title, body });
        notify("ok", "草稿已创建");
        onDone();
      } catch (problem) {
        notify("bad", "创建草稿失败", problem instanceof Error ? problem.message : undefined);
      } finally {
        setBusy(false);
      }
    })()}>{busy ? "创建中…" : "创建草稿"}</button></>}>
      <div className="stack" style={{ gap: 12 }}>
        <Field label="候选">
          <select className="select" value={candidateId} onChange={(event) => setCandidateId(event.target.value)}>
            {candidates.map((candidate) => <option key={candidate.id} value={candidate.id}>{candidate.title}</option>)}
          </select>
        </Field>
        <Field label="标题"><input className="input" value={title} onChange={(event) => setTitle(event.target.value)} /></Field>
        <Field label="正文"><textarea className="textarea" style={{ minHeight: 220 }} value={body} onChange={(event) => setBody(event.target.value)} /></Field>
      </div>
    </Modal>
  );
}

function PublishModal({ draft, channels, onClose, onDone }: { draft: Draft; channels: Channel[]; onClose: () => void; onDone: () => void }) {
  const { notify, run } = useUi();
  const [selected, setSelected] = useState<string[]>(channels.filter((channel) => channel.enabled).map((channel) => channel.id));
  const [scheduledAt, setScheduledAt] = useState("");
  const [busy, setBusy] = useState(false);

  return (
    <Modal title="发布草稿" sub={draft.title} onClose={onClose} footer={<><button type="button" className="btn" onClick={onClose}>取消</button><button type="button" className="btn btn-primary" disabled={busy || selected.length === 0} onClick={() => void (async () => {
      setBusy(true);
      let started = false;
      try {
        const operation = await createPublication({ draftId: draft.id, channelIds: selected, scheduledAt: scheduledAt || null });
        started = true;
        await run(operation, "发布");
        onDone();
      } catch (problem) {
        if (!started) {
          notify("bad", "发布失败", problem instanceof Error ? problem.message : undefined);
        }
      } finally {
        setBusy(false);
      }
    })()}>{busy ? "发布中…" : "创建发布"}</button></>}>
      <div className="stack" style={{ gap: 12 }}>
        <Field label="渠道">
          <div className="publish-grid">
            {channels.map((channel) => (
              <label className="domain-option" key={channel.id}>
                <input type="checkbox" disabled={!channel.enabled} checked={selected.includes(channel.id)} onChange={(event) => setSelected((current) => event.target.checked ? [...current, channel.id] : current.filter((id) => id !== channel.id))} />
                <span>{channel.name} · {channel.kind}</span>
              </label>
            ))}
          </div>
        </Field>
        <Field label="计划时间" hint="留空立即发布；渠道独立投递，单个失败不影响其他结果">
          <input className="input" type="datetime-local" value={scheduledAt} onChange={(event) => setScheduledAt(event.target.value)} />
        </Field>
      </div>
    </Modal>
  );
}

function RunDrawer({ run: initial, onClose }: { run: ProductionRun; onClose: () => void }) {
  const [run, setRun] = useState(initial);
  const [attempts, setAttempts] = useState<Attempt[]>([]);
  useEffect(() => {
    void (async () => {
      try {
        const [fresh, page] = await Promise.all([getProductionRun(initial.id), listRunAttempts(initial.id)]);
        setRun(fresh);
        setAttempts(page.items);
      } catch {
        // 保留初始数据。
      }
    })();
  }, [initial.id]);
  return (
    <Drawer title={`运行 ${run.id}`} sub={`${run.mode} · ${run.status}`} onClose={onClose} wide footer={<button type="button" className="btn" onClick={onClose}>关闭</button>}>
      <div className="stack" style={{ gap: 14 }}>
        <dl className="kv">
          <dt>输入集</dt><dd className="mono">{run.inputSetId}</dd>
          <dt>工作流版本</dt><dd className="mono">{run.workflowVersionId}</dd>
          <dt>任务</dt><dd className="mono">{run.taskId}</dd>
          <dt>候选</dt><dd className="mono">{run.candidateIds.join("、") || "—"}</dd>
          <dt>创建</dt><dd>{stamp(run.createdAt)}</dd>
          <dt>更新</dt><dd>{stamp(run.updatedAt)}</dd>
        </dl>
        {attempts.map((attempt) => (
          <div className="drawer-section" key={attempt.id}>
            <h3>Attempt {attempt.number} · {attempt.status}</h3>
            <div className="timeline">
              {attempt.steps.map((step) => (
                <div className="tl-item" key={step.id}>
                  <div className="tl-rail"><span className={`tl-dot ${step.status === "succeeded" ? "ok" : step.status === "failed" ? "bad" : "warn"}`} /><span className="tl-line" /></div>
                  <div className="tl-body">
                    <div className="tl-title">{step.id} · {step.kind}</div>
                    <div className="tl-meta">{step.ms}ms · {step.status}</div>
                    {step.output ? <div className="code" style={{ marginTop: 6 }}>{step.output}</div> : null}
                    {step.error ? <div className="small" style={{ color: "var(--danger)" }}>{step.error}</div> : null}
                  </div>
                </div>
              ))}
            </div>
            {attempt.error ? <div className="alert bad">{attempt.error}</div> : null}
          </div>
        ))}
      </div>
    </Drawer>
  );
}

function CandidateDrawer({ candidate, onClose, onDone }: { candidate: Candidate; onClose: () => void; onDone: () => void }) {
  const { notify } = useUi();
  const [comment, setComment] = useState("");
  const [busy, setBusy] = useState(false);
  async function decide(decision: string) {
    setBusy(true);
    try {
      await reviewCandidate(candidate.id, candidate.etag ?? "", { decision, comment });
      notify("ok", "审核决定已记录");
      onDone();
    } catch (problem) {
      notify("bad", "审核失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Drawer
      title={candidate.title}
      sub={`候选 · ${candidate.status}`}
      onClose={onClose}
      wide
      footer={<><button type="button" className="btn" onClick={onClose}>关闭</button><button type="button" className="btn" disabled={busy} onClick={() => void decide("returnForRegeneration")}>退回重生成</button><button type="button" className="btn btn-danger" disabled={busy} onClick={() => void decide("reject")}><IconClose size={13} /> 驳回</button><button type="button" className="btn btn-primary" disabled={busy || candidate.validation.issues.length > 0} onClick={() => void decide("approve")}><IconCheck size={13} /> 批准</button></>}
    >
      <div className="stack" style={{ gap: 14 }}>
        <div className="detail-meta"><StatusBadge status={candidate.status} /><span className="mono">{candidate.id}</span><span>运行 {candidate.runId}</span></div>
        {candidate.validation.issues.length ? <div className="alert bad">{candidate.validation.issues.map((issue) => issue.message).join("；")}</div> : null}
        <div className="body-text">{candidate.body.split(/\n{2,}/).map((paragraph, index) => <p key={index}>{paragraph}</p>)}</div>
        <Field label="审核意见"><textarea className="textarea" value={comment} onChange={(event) => setComment(event.target.value)} /></Field>
        <div className="code">{JSON.stringify(candidate.outputs, null, 2)}</div>
      </div>
    </Drawer>
  );
}

function DraftDrawer({ draft, onClose, onDone }: { draft: Draft; onClose: () => void; onDone: () => void }) {
  const { notify } = useUi();
  const [title, setTitle] = useState(draft.title);
  const [body, setBody] = useState(draft.body);
  const [busy, setBusy] = useState(false);
  return (
    <Drawer
      title={`编辑草稿 · v${draft.latestVersion}`}
      sub={draft.id}
      onClose={onClose}
      wide
      footer={<><button type="button" className="btn" onClick={onClose}>关闭</button><button type="button" className="btn btn-primary" disabled={busy} onClick={() => void (async () => {
        setBusy(true);
        try {
          await updateDraft(draft.id, { candidateIds: draft.candidateIds, title, body }, draft.etag ?? "");
          notify("ok", "草稿已保存为新版本");
          onDone();
        } catch (problem) {
          notify("bad", "保存草稿失败", problem instanceof Error ? problem.message : undefined);
        } finally {
          setBusy(false);
        }
      })()}>保存新版本</button></>}
    >
      <div className="stack" style={{ gap: 12 }}>
        <Field label="标题"><input className="input" value={title} onChange={(event) => setTitle(event.target.value)} /></Field>
        <Field label="正文"><textarea className="textarea" style={{ minHeight: 320 }} value={body} onChange={(event) => setBody(event.target.value)} /></Field>
        <div className="small muted">版本历史 {draft.versions.length} 条；保存不会覆盖旧版本。</div>
      </div>
    </Drawer>
  );
}
