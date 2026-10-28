import { useCallback, useEffect, useState } from "react";

import { IconPlus, IconRefresh } from "../components/icons";
import { Badge, Card, Empty, Field, Loading, Problem, useUi } from "../components/ui";
import {
  bindProjectResource,
  checkWizardStep,
  completeWizardStep,
  createProject,
  createWizard,
  dismissWizard,
  fetchUrl,
  getPreconditions,
  getWizard,
  listProjectResources,
  listProjects,
  listSearchEngines,
  parseInput,
  runSearch,
  skipWizardStep,
} from "../lib/api";
import { stamp } from "../lib/format";
import { useCurrentProject } from "../lib/project";
import type {
  FetchResult,
  PreconditionReport,
  Project,
  ProjectResource,
  SearchEngine,
  SearchResult,
  WizardCheck,
  WizardDoc,
} from "../lib/types";

const STEP_TITLES: Record<string, string> = {
  create_project: "建项目",
  scenario_template: "选择场景模板",
  configure_sources: "配置信源",
  verify_collection: "校验采集",
  model_gate: "模型与依赖门闩",
  processing_output: "加工与产出",
  api_surface: "对外调用说明",
};

export function ProjectsPage() {
  const { notify, run } = useUi();
  const { setCurrentProjectId } = useCurrentProject();
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [name, setName] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const page = await listProjects();
      setProjects(page.items);
      setSelected((current) => current ?? page.items.find((item) => item.isDefault)?.id ?? page.items[0]?.id ?? null);
      setError(null);
    } catch (problem) {
      setError(problem);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function submit() {
    const trimmed = name.trim();
    if (!trimmed) return;
    setBusy(true);
    try {
      const project = await createProject({ name: trimmed });
      setName("");
      notify("ok", "项目已创建", project.id);
      await load();
      setSelected(project.id);
    } catch (problem) {
      notify("bad", "创建项目失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <div className="kicker">projects · 项目</div>
          <h1 className="page-title">按项目归属资源，向导带你走完首条主线</h1>
          <p className="page-lede">
            旧全局数据由默认项目承接：未归属的信源在默认项目视角下软接展示，归属后即从软接清单消失。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={14} /> 刷新
          </button>
        </div>
      </header>

      {error ? <Problem error={error} onRetry={() => void load()} /> : null}

      <div className="grid cols-2">
        <Card title="新建项目" sub="项目名必填；描述可选。">
          <form
            className="stack"
            onSubmit={(event) => {
              event.preventDefault();
              void submit();
            }}
          >
            <Field label="项目名" htmlFor="project-name">
              <input
                id="project-name"
                className="input"
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="例如：科技快讯"
                disabled={busy}
              />
            </Field>
            <div>
              <button type="submit" className="btn btn-primary" disabled={busy || !name.trim()}>
                <IconPlus size={14} /> 创建
              </button>
            </div>
          </form>
        </Card>

        <Card title="项目" sub={projects ? `${projects.length} 个` : undefined} flush>
          {loading && !projects ? (
            <Loading lines={3} />
          ) : !projects?.length ? (
            <Empty title="还没有项目" text="默认项目会在服务启动时自动创建。" />
          ) : (
            <div className="list">
              {projects.map((project) => (
                <button
                  type="button"
                  key={project.id}
                  className={`list-row${selected === project.id ? " is-active" : ""}`}
                  onClick={() => {
                    setSelected(project.id);
                    setCurrentProjectId(project.id);
                  }}
                >
                  <div className="stack" style={{ gap: 2 }}>
                    <strong>
                      {project.name}
                      {project.isDefault ? (
                        <>
                          {" "}
                          <Badge tone="neutral">默认</Badge>
                        </>
                      ) : null}
                    </strong>
                    <span className="small muted">
                      {project.id} · 创建于 {stamp(project.createdAt)}
                    </span>
                  </div>
                </button>
              ))}
            </div>
          )}
        </Card>
      </div>

      {selected ? (
        <>
          <WizardPanel projectId={selected} onNotify={notify} onRun={run} />
          <TryPanel projectId={selected} onNotify={notify} />
        </>
      ) : null}
    </div>
  );
}

function WizardPanel({
  projectId,
  onNotify,
  onRun,
}: {
  projectId: string;
  onNotify: UiNotify;
  onRun: UiRun;
}) {
  const [wizard, setWizard] = useState<WizardDoc | null>(null);
  const [resources, setResources] = useState<ProjectResource[]>([]);
  const [check, setCheck] = useState<WizardCheck | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [bindKind, setBindKind] = useState("source");
  const [bindId, setBindId] = useState("");

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [doc, res] = await Promise.all([
        getWizard(projectId),
        listProjectResources(projectId).catch(() => ({ items: [] }) as unknown as { items: ProjectResource[] }),
      ]);
      setWizard(doc);
      setResources(res.items);
      setCheck(null);
      setError(null);
    } catch (problem) {
      setError(problem);
    } finally {
      setLoading(false);
    }
  }, [projectId]);

  useEffect(() => {
    void load();
  }, [load]);

  async function act(label: string, action: () => Promise<unknown>) {
    try {
      const operation = await action();
      await onRun(operation as never, label);
      await load();
    } catch (problem) {
      onNotify("bad", `${label}失败`, problem instanceof Error ? problem.message : undefined);
    }
  }

  async function checkStep(stepId: string) {
    try {
      const result = await checkWizardStep(projectId, stepId);
      setCheck(result);
      setWizard(result.wizard);
      if (result.met) onNotify("ok", "环节条件满足", `${STEP_TITLES[stepId] ?? stepId} 已置为完成`);
      else onNotify("bad", "环节条件未满足", result.detail?.where ?? result.detail?.reason);
    } catch (problem) {
      onNotify("bad", "校验失败", problem instanceof Error ? problem.message : undefined);
    }
  }

  const closed = wizard?.status === "completed" || wizard?.status === "dismissed";

  return (
    <Card
      title={`向导 · ${STEP_TITLES[wizard?.steps[0]?.id ?? ""] ?? "标准主线"}`}
      sub={
        wizard
          ? wizard.status === "dismissed"
            ? "已跳过：向导不再打扰，可重新建档。"
            : wizard.status === "completed"
              ? "全部环节完成，向导已收口。"
              : "完成或跳过全部环节后向导收口。"
          : undefined
      }
      action={
        <div className="row" style={{ gap: 8 }}>
          {!wizard ? (
            <button type="button" className="btn btn-primary" onClick={() => void act("开始向导", () => createWizard(projectId))}>
              开始向导
            </button>
          ) : (
            <button type="button" className="btn" onClick={() => void act("跳过向导", () => dismissWizard(projectId))} disabled={closed}>
              跳过向导
            </button>
          )}
          <button type="button" className="btn" onClick={() => void load()}>
            <IconRefresh size={14} /> 刷新
          </button>
        </div>
      }
    >
      {error ? <Problem error={error} onRetry={() => void load()} /> : null}
      {loading && !wizard ? (
        <Loading lines={3} />
      ) : !wizard ? (
        <Empty title="尚未建档" text="建档后按环节推进：配置信源 → 校验采集 → 模型门闩 → 加工产出。" />
      ) : (
        <div className="stack">
          <div className="list">
            {wizard.steps.map((step) => (
              <div className="list-row" key={step.id}>
                <div className="stack" style={{ gap: 2 }}>
                  <strong>{STEP_TITLES[step.id] ?? step.id}</strong>
                  {check?.stepId === step.id && !check.met ? (
                    <span className="small muted">
                      校验未通过：{check.detail?.where ?? check.detail?.reason ?? "手动完成"}
                    </span>
                  ) : null}
                </div>
                <div className="row" style={{ gap: 6 }}>
                  <Badge tone={step.status === "done" ? "ok" : step.status === "skipped" ? "warn" : "neutral"}>
                    {step.status === "done" ? "完成" : step.status === "skipped" ? "跳过" : "待办"}
                  </Badge>
                  {!closed && step.status === "pending" ? (
                    <>
                      <button type="button" className="btn btn-sm" onClick={() => void checkStep(step.id)}>
                        校验
                      </button>
                      <button
                        type="button"
                        className="btn btn-sm"
                        onClick={() => void act(`完成 ${STEP_TITLES[step.id] ?? step.id}`, () => completeWizardStep(projectId, step.id))}
                      >
                        完成
                      </button>
                      <button
                        type="button"
                        className="btn btn-sm"
                        onClick={() => void act(`跳过 ${STEP_TITLES[step.id] ?? step.id}`, () => skipWizardStep(projectId, step.id))}
                      >
                        跳过
                      </button>
                    </>
                  ) : null}
                </div>
              </div>
            ))}
          </div>

          <div className="stack">
            <div className="card-title">资源归属（{resources.length}）</div>
            {resources.length ? (
              <div className="mono small">
                {resources.map((resource) => `${resource.kind}:${resource.id}${resource.legacy ? "（软接）" : ""}`).join("、")}
              </div>
            ) : (
              <span className="small muted">尚无归属资源。信源可在信源页创建后在此归属。</span>
            )}
            {!closed ? (
              <div className="row" style={{ gap: 8 }}>
                <select className="input" style={{ width: 160 }} value={bindKind} onChange={(event) => setBindKind(event.target.value)}>
                  {["source", "task", "content", "workflow_version", "publication"].map((kind) => (
                    <option key={kind} value={kind}>
                      {kind}
                    </option>
                  ))}
                </select>
                <input
                  className="input"
                  style={{ width: 260 }}
                  value={bindId}
                  onChange={(event) => setBindId(event.target.value)}
                  placeholder="资源 ID"
                />
                <button
                  type="button"
                  className="btn btn-sm"
                  disabled={!bindId.trim()}
                  onClick={() =>
                    void act("归属资源", () => bindProjectResource(projectId, bindKind, bindId.trim()))
                  }
                >
                  归属
                </button>
              </div>
            ) : null}
          </div>
        </div>
      )}
    </Card>
  );
}

function TryPanel({
  projectId,
  onNotify,
}: {
  projectId: string;
  onNotify: UiNotify;
}) {
  const [engines, setEngines] = useState<SearchEngine[]>([]);
  const [engine, setEngine] = useState("");
  const [query, setQuery] = useState("");
  const [searchResult, setSearchResult] = useState<SearchResult | null>(null);
  const [url, setUrl] = useState("");
  const [fetchResult, setFetchResult] = useState<FetchResult | null>(null);
  const [parseResult, setParseResult] = useState<Record<string, unknown> | null>(null);
  const [preconditions, setPreconditions] = useState<PreconditionReport | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void listSearchEngines()
      .then((page) => setEngines(page.items))
      .catch(() => setEngines([]));
    void getPreconditions()
      .then(setPreconditions)
      .catch(() => setPreconditions(null));
  }, []);

  async function search() {
    const trimmed = query.trim();
    if (!trimmed) return;
    setBusy(true);
    try {
      setSearchResult(await runSearch({ query: trimmed, engine: engine || undefined, projectId, limit: 10 }));
    } catch (problem) {
      setSearchResult(null);
      onNotify("bad", "试搜失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  async function fetchPage() {
    const trimmed = url.trim();
    if (!trimmed) return;
    setBusy(true);
    try {
      setFetchResult(await fetchUrl(trimmed));
    } catch (problem) {
      setFetchResult(null);
      onNotify("bad", "获取失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  async function parsePage() {
    const trimmed = url.trim();
    if (!trimmed) return;
    setBusy(true);
    try {
      setParseResult(await parseInput({ url: trimmed }));
      onNotify("ok", "解析完成");
    } catch (problem) {
      setParseResult(null);
      onNotify("bad", "解析失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="grid cols-2">
      <Card title="试搜" sub="走产品搜索路径：前置未满足时返回 412 与“去哪配”。">
        <div className="stack">
          <div className="row" style={{ gap: 8 }}>
            <select className="input" style={{ width: 200 }} value={engine} onChange={(event) => setEngine(event.target.value)}>
              <option value="">默认引擎</option>
              {engines.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.name}
                  {item.requiresKeyRef ? "（需密钥）" : ""}
                </option>
              ))}
            </select>
            <input
              className="input"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="搜索关键词"
              onKeyDown={(event) => {
                if (event.key === "Enter") void search();
              }}
            />
            <button type="button" className="btn btn-primary" onClick={() => void search()} disabled={busy}>
              搜索
            </button>
          </div>
          {preconditions && !preconditions.met ? (
            <div className="small muted">
              前置未就绪：
              {preconditions.scopes
                .filter((item) => !item.met)
                .map((item) => item.requirements.map((req) => `${req.message}（${req.where}）`).join("；"))
                .join("；")}
            </div>
          ) : null}
          {searchResult ? (
            <div className="stack" style={{ gap: 6 }}>
              <span className="small muted">
                引擎 {searchResult.engine} · {searchResult.count} 条结果
              </span>
              {searchResult.results.map((hit, index) => (
                <div key={index} className="stack" style={{ gap: 1 }}>
                  <a href={hit.url ?? "#"} target="_blank" rel="noreferrer" className="mono small">
                    {hit.title ?? hit.url ?? "(无标题)"}
                  </a>
                  {hit.snippet ? <span className="small muted">{hit.snippet}</span> : null}
                </div>
              ))}
            </div>
          ) : null}
        </div>
      </Card>

      <Card title="试取正文" sub="POST /fetch：URL → 正文，回环地址也可用于本机验证。">
        <div className="stack">
          <div className="row" style={{ gap: 8 }}>
            <input
              className="input"
              value={url}
              onChange={(event) => setUrl(event.target.value)}
              placeholder="https://example.com/page"
              onKeyDown={(event) => {
                if (event.key === "Enter") void fetchPage();
              }}
            />
            <button type="button" className="btn btn-primary" onClick={() => void fetchPage()} disabled={busy}>
              获取
            </button>
          </div>
          {fetchResult ? (
            <div className="stack" style={{ gap: 4 }}>
              <span className="small muted">
                HTTP {fetchResult.status} · {fetchResult.contentType || "未知类型"}
                {fetchResult.truncated ? " · 已截断" : ""}
              </span>
              <pre className="mono small" style={{ maxHeight: 240, overflow: "auto", whiteSpace: "pre-wrap" }}>
                {fetchResult.text.slice(0, 4000)}
              </pre>
            </div>
          ) : null}
        </div>
      </Card>

      <Card title="试解析" sub="POST /parse：URL → 结构化抽取（rawprism 未接入时返回 412）。">
        <div className="stack">
          <div className="row" style={{ gap: 8 }}>
            <input
              className="input"
              value={url}
              onChange={(event) => setUrl(event.target.value)}
              placeholder="https://example.com/page"
              onKeyDown={(event) => {
                if (event.key === "Enter") void parsePage();
              }}
            />
            <button type="button" className="btn btn-primary" onClick={() => void parsePage()} disabled={busy}>
              解析
            </button>
          </div>
          {parseResult ? (
            <pre className="mono small" style={{ maxHeight: 240, overflow: "auto", whiteSpace: "pre-wrap" }}>
              {JSON.stringify(parseResult, null, 2).slice(0, 4000)}
            </pre>
          ) : null}
        </div>
      </Card>
    </div>
  );
}

type UiNotify = (tone: "ok" | "bad", title: string, detail?: string) => void;
type UiRun = (operation: never, label: string) => Promise<unknown>;
