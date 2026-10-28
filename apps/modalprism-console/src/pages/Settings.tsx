import { useCallback, useEffect, useMemo, useState } from "react";

import { IconCheck, IconPlus, IconRefresh } from "../components/icons";
import { Card, Empty, Field, Loading, Modal, Problem, StatusBadge, useUi } from "../components/ui";
import {
  applySettings,
  createFactoryReset,
  createModel,
  createPrompt,
  createPromptVersion,
  createReset,
  createSkill,
  createSkillVersion,
  diffSettings,
  getEffectiveSettings,
  getSettingsSchema,
  listModels,
  listPrompts,
  listSkills,
  previewFactoryReset,
  previewReset,
  rollbackSettings,
  settingsHistory,
  validateSettings,
} from "../lib/api";
import { stamp } from "../lib/format";
import type {
  EffectiveSettings,
  ModelResource,
  PromptResource,
  ResetPreview,
  SettingsField,
  SettingsGroup,
  SettingsHistoryEntry,
  SettingsSchema,
} from "../lib/types";

type Section = string;

const DOMAINS = [
  ["tasks", "任务", "排队、活动与终态任务"],
  ["content", "内容", "版本、关系、血缘与快照"],
  ["blobs", "对象", "无引用的本地或外部对象"],
  ["derivedData", "派生数据", "全文、向量等派生索引"],
  ["production", "生产", "输入集、运行、候选、草稿与发布"],
  ["staging", "暂存", "未提交对象与过期上传"],
  ["logs", "日志", "普通日志与结构化事件"],
  ["audit", "审计", "不可修改的治理事件"],
  ["operations", "操作", "已终结的 Operation"],
  ["userSettings", "用户设置", "模型、提示词、Skill 与流程"],
] as const;

export function SettingsPage() {
  const { notify, run } = useUi();
  const [schema, setSchema] = useState<SettingsSchema | null>(null);
  const [effective, setEffective] = useState<EffectiveSettings | null>(null);
  const [etag, setEtag] = useState("");
  const [history, setHistory] = useState<SettingsHistoryEntry[]>([]);
  const [models, setModels] = useState<ModelResource[]>([]);
  const [prompts, setPrompts] = useState<PromptResource[]>([]);
  const [skills, setSkills] = useState<PromptResource[]>([]);
  const [section, setSection] = useState<Section>("general");
  const [dirty, setDirty] = useState<Record<string, unknown>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [diff, setDiff] = useState<Record<string, unknown> | null>(null);
  const [modelOpen, setModelOpen] = useState(false);
  const [promptOpen, setPromptOpen] = useState<"prompt" | "skill" | null>(null);
  const [versionFor, setVersionFor] = useState<PromptResource | null>(null);
  const [preview, setPreview] = useState<ResetPreview | null>(null);
  const [factory, setFactory] = useState(false);
  const [reportId, setReportId] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [schemaValue, effectiveValue, historyPage, modelPage, promptPage, skillPage] = await Promise.all([
        getSettingsSchema(),
        getEffectiveSettings(),
        settingsHistory({ limit: 50 }),
        listModels(),
        listPrompts(),
        listSkills(),
      ]);
      setSchema(schemaValue);
      setEffective(effectiveValue.data);
      setEtag(effectiveValue.etag ?? "");
      setHistory(historyPage.items);
      setModels(modelPage.items);
      setPrompts(promptPage.items);
      setSkills(skillPage.items);
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

  const groups: SettingsGroup[] = (schema?.groups ?? []).filter((group) => group.id !== "maintenance");
  const fields = useMemo(() => (schema?.fields ?? []).filter((field) => field.group === section), [schema, section]);
  const maintenanceFields = useMemo(
    () => (schema?.fields ?? []).filter((field) => field.group === "maintenance"),
    [schema],
  );
  const dirtyKeys = Object.keys(dirty);

  function update(field: SettingsField, value: unknown) {
    setDirty((current) => ({ ...current, [field.key]: value }));
    setErrors((current) => {
      const next = { ...current };
      delete next[field.key];
      return next;
    });
  }

  async function validate(): Promise<boolean> {
    setBusy(true);
    setErrors({});
    try {
      const result = (await validateSettings(dirty)) as { valid: boolean; fieldErrors: { field: string; message: string }[] };
      const next: Record<string, string> = {};
      for (const item of result.fieldErrors ?? []) next[item.field] = item.message;
      setErrors(next);
      if (!result.valid) {
        notify("bad", "校验未通过", "字段错误已标在对应输入下方。");
        return false;
      }
      notify("ok", "校验通过");
      return true;
    } catch (problem) {
      notify("bad", "校验失败", problem instanceof Error ? problem.message : undefined);
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function showDiff() {
    setBusy(true);
    try {
      setDiff((await diffSettings(dirty)) as Record<string, unknown>);
    } catch (problem) {
      notify("bad", "生成差异失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  async function apply() {
    setBusy(true);
    try {
      if (!(await validate())) return;
      const operation = await applySettings(dirty, "控制台应用设置", etag);
      await run(operation, "应用系统设置");
      setDirty({});
      setDiff(null);
      await load();
    } catch (problem) {
      notify("bad", "应用设置失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  async function rollback(target: number) {
    const reason = window.prompt(`回滚到 g${target} 的理由`);
    if (!reason) return;
    setBusy(true);
    try {
      const operation = await rollbackSettings(target, reason, etag);
      await run(operation, "回滚设置");
      await load();
    } catch (problem) {
      notify("bad", "回滚失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  async function previewResetDomains(domains: string[]) {
    setBusy(true);
    try {
      const result = await previewReset(domains);
      setPreview(result.data);
    } catch (problem) {
      notify("bad", "预演失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  async function previewFactory() {
    setBusy(true);
    try {
      const result = await previewFactoryReset();
      setPreview(result.data);
      setFactory(true);
    } catch (problem) {
      notify("bad", "预演失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  async function confirmReset() {
    if (!preview) return;
    setBusy(true);
    let started = false;
    try {
      const operation = factory
        ? await createFactoryReset(preview, preview.confirmationToken)
        : await createReset(preview, preview.confirmationToken);
      started = true;
      const finished = await run(operation, factory ? "恢复出厂设置" : "分域清空");
      const summary = (finished.resultSummary ?? {}) as Record<string, unknown>;
      setReportId(String(summary.reportId ?? "") || null);
      setPreview(null);
      setFactory(false);
      await load();
    } catch (problem) {
      if (!started) {
        notify("bad", "执行失败", problem instanceof Error ? problem.message : undefined);
      }
    } finally {
      setBusy(false);
    }
  }

  if (loading && !schema) {
    return <div className="page"><Loading lines={6} /></div>;
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <div className="kicker">settings · 系统设置</div>
          <h1 className="page-title">集中、可校验、可回滚的运行策略</h1>
          <p className="page-lede">
            设置是面向用户的有效值，不是配置文件的镜像。每次应用生成新代际；模型、提示词与 Skill 都是版本化资源，生成物在批准前不会启用。
          </p>
        </div>
        <div className="page-actions">
          <StatusBadge status={effective ? "online" : "degraded"} />
          <span className="small muted">当前代际 <strong className="mono">G-{effective?.generation ?? "—"}</strong></span>
          <button type="button" className="btn" onClick={() => void load()} disabled={loading}>
            <IconRefresh size={14} /> 刷新
          </button>
        </div>
      </header>

      {error ? <Problem error={error} onRetry={() => void load()} /> : null}

      {dirtyKeys.length > 0 ? (
        <div className="batchbar">
          <span>{dirtyKeys.length} 项未应用</span>
          <button type="button" className="btn btn-sm" disabled={busy} onClick={() => void validate()}>校验</button>
          <button type="button" className="btn btn-sm" disabled={busy} onClick={() => void showDiff()}>查看差异</button>
          <button type="button" className="btn btn-sm" onClick={() => setDirty({})}>放弃</button>
          <span className="fill" />
          <button type="button" className="btn btn-primary btn-sm" disabled={busy} onClick={() => void apply()}>应用设置</button>
        </div>
      ) : null}

      <div className="settings-layout">
        <nav className="settings-nav" aria-label="设置分组">
          {groups.map((group) => (
            <button key={group.id} type="button" className={section === group.id ? "is-active" : ""} onClick={() => setSection(group.id)}>
              {group.label}
            </button>
          ))}
          <span className="nav-label">资源</span>
          <button type="button" className={section === "models" ? "is-active" : ""} onClick={() => setSection("models")}>模型</button>
          <button type="button" className={section === "prompts" ? "is-active" : ""} onClick={() => setSection("prompts")}>提示词</button>
          <button type="button" className={section === "skills" ? "is-active" : ""} onClick={() => setSection("skills")}>Skill</button>
          <button type="button" className={section === "history" ? "is-active" : ""} onClick={() => setSection("history")}>变更历史</button>
          <button type="button" className={section === "maintenance" ? "is-active" : ""} onClick={() => setSection("maintenance")}>维护与重置</button>
        </nav>

        <div className="stack" style={{ gap: 16 }}>
          {groups.some((group) => group.id === section) ? (
            <Card title={groups.find((group) => group.id === section)?.label} sub={groups.find((group) => group.id === section)?.note}>
              {fields.map((field) => (
                <SettingRow
                  key={field.key}
                  field={field}
                  value={dirty[field.key] ?? effective?.values[field.key] ?? field.default}
                  error={errors[field.key]}
                  onChange={(value) => update(field, value)}
                />
              ))}
            </Card>
          ) : null}

          {section === "maintenance" ? (
            <Card title="维护参数" sub="排空超时与确认令牌有效期；清空与恢复出厂见下方">
              {maintenanceFields.map((field) => (
                <SettingRow
                  key={field.key}
                  field={field}
                  value={dirty[field.key] ?? effective?.values[field.key] ?? field.default}
                  error={errors[field.key]}
                  onChange={(value) => update(field, value)}
                />
              ))}
            </Card>
          ) : null}

          {section === "models" ? (
            <Card title="模型资源" sub="密钥只保存环境变量引用；删除被版本引用的模型会被拒绝" action={<button type="button" className="btn btn-sm" onClick={() => setModelOpen(true)}><IconPlus size={12} /> 登记模型</button>} flush>
              {models.length === 0 ? <Empty title="还没有模型" text="登记一个 OpenAI 兼容端点、本机服务或命令行模型。" /> : (
                <div className="list">
                  {models.map((model) => (
                    <div className="list-row" key={model.id}>
                      <span className="list-main">
                        <span className="list-title">{model.name}</span>
                        <span className="list-meta">
                          <span className="mono">{model.id}</span>
                          <span>{model.kind}</span>
                          <span>{model.model}</span>
                          <span>{model.secretRef ? `密钥 ${model.secretRef}${model.secretPresent ? "（存在）" : "（缺失）"}` : "无密钥"}</span>
                        </span>
                      </span>
                      <span className={`badge ${model.enabled ? "ok" : "neutral"}`}>{model.enabled ? "启用" : "停用"}</span>
                    </div>
                  ))}
                </div>
              )}
            </Card>
          ) : null}

          {section === "prompts" || section === "skills" ? (
            <Card
              title={section === "prompts" ? "提示词" : "Skill"}
              sub="稳定身份 + 版本；被运行引用的版本不可修改"
              action={<button type="button" className="btn btn-sm" onClick={() => setPromptOpen(section === "prompts" ? "prompt" : "skill")}><IconPlus size={12} /> 新建</button>}
              flush
            >
              {(section === "prompts" ? prompts : skills).length === 0 ? <Empty title="还没有内容" /> : (
                <div className="list">
                  {(section === "prompts" ? prompts : skills).map((item) => (
                    <div className="list-row" key={item.id}>
                      <span className="list-main">
                        <span className="list-title">{item.name}{item.builtin ? <span className="tag" style={{ marginLeft: 6 }}>内置</span> : null}</span>
                        <span className="list-meta"><span>{item.description}</span><span>{item.versions.length} 个版本</span><span className="mono">{item.latestVersionId}</span></span>
                      </span>
                      <button type="button" className="btn btn-sm" onClick={() => setVersionFor(item)}>新版本</button>
                    </div>
                  ))}
                </div>
              )}
            </Card>
          ) : null}

          {section === "history" ? (
            <Card title="变更历史" sub="回滚创建新代际，不删除历史" flush>
              {history.length === 0 ? <Empty title="还没有历史" /> : (
                <div className="table-wrap">
                  <table className="table">
                    <thead><tr><th>代际</th><th>摘要</th><th>应用时间</th><th>回滚来源</th><th /></tr></thead>
                    <tbody>
                      {history.map((item) => (
                        <tr key={item.generation}>
                          <td className="mono">G-{item.generation}</td>
                          <td>{item.summary}</td>
                          <td className="small muted">{stamp(item.appliedAt)}</td>
                          <td className="mono small">{item.rollbackOf ? `G-${item.rollbackOf}` : "—"}</td>
                          <td className="actions">
                            <button type="button" className="btn btn-sm" disabled={busy || item.generation === effective?.generation} onClick={() => void rollback(item.generation)}>回滚至此</button>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </Card>
          ) : null}

          {section === "maintenance" ? (
            <MaintenanceCard
              busy={busy}
              onPreview={(domains) => void previewResetDomains(domains)}
              onFactory={() => void previewFactory()}
              reportId={reportId}
            />
          ) : null}
        </div>
      </div>

      {diff ? <DiffModal diff={diff} onClose={() => setDiff(null)} onApply={() => { setDiff(null); void apply(); }} /> : null}
      {modelOpen ? <ModelModal onClose={() => setModelOpen(false)} onDone={() => { setModelOpen(false); void load(); }} /> : null}
      {promptOpen ? <NamedModal kind={promptOpen} onClose={() => setPromptOpen(null)} onDone={() => { setPromptOpen(null); void load(); }} /> : null}
      {versionFor ? <VersionModal resource={versionFor} onClose={() => setVersionFor(null)} onDone={() => { setVersionFor(null); void load(); }} /> : null}
      {preview ? (
        <ResetModal
          preview={preview}
          factory={factory}
          onClose={() => { setPreview(null); setFactory(false); }}
          onConfirm={() => void confirmReset()}
        />
      ) : null}
    </div>
  );
}

function SettingRow({
  field,
  value,
  error,
  onChange,
}: {
  field: SettingsField;
  value: unknown;
  error?: string;
  onChange: (value: unknown) => void;
}) {
  return (
    <div className="field-row">
      <div className="field-copy">
        <span className="field-label">{field.label}</span>
        <span className="field-hint">{field.description}</span>
        <span className="field-key">
          {field.key} · {field.effect === "restartRequired" ? "需重启" : "热加载"}
          {field.editable ? "" : " · 固定"}
        </span>
      </div>
      <SettingControl field={field} value={value} error={error} onChange={onChange} />
    </div>
  );
}

function SettingControl({
  field,
  value,
  error,
  onChange,
}: {
  field: SettingsField;
  value: unknown;
  error?: string;
  onChange: (value: unknown) => void;
}) {
  if (field.type === "boolean") {
    return (
      <label className="switch">
        <input type="checkbox" checked={Boolean(value)} disabled={!field.editable} onChange={(event) => onChange(event.target.checked)} />
        <span>{value ? "开启" : "关闭"}</span>
        {error ? <span className="field-error">{error}</span> : null}
      </label>
    );
  }
  if (field.type === "integer" || field.type === "number") {
    return (
      <div className="stack" style={{ gap: 4 }}>
        <div className="row">
          <input
            className="input"
            type="number"
            min={field.minimum}
            max={field.maximum}
            step={field.type === "integer" ? 1 : 0.1}
            value={String(value ?? "")}
            disabled={!field.editable}
            onChange={(event) => onChange(field.type === "integer" ? Number.parseInt(event.target.value, 10) || 0 : Number(event.target.value))}
          />
          {field.unit ? <span className="muted small">{field.unit}</span> : null}
          {field.presets.length > 0 ? (
            <select
              className="select"
              value=""
              disabled={!field.editable}
              aria-label={`${field.label}常用值`}
              onChange={(event) => {
                if (!event.target.value) return;
                onChange(field.type === "integer" ? Number.parseInt(event.target.value, 10) : Number(event.target.value));
              }}
            >
              <option value="">常用值…</option>
              {field.presets.map((preset) => (
                <option key={preset.value} value={preset.value}>{preset.label}</option>
              ))}
            </select>
          ) : null}
        </div>
        {error ? <span className="field-error">{error}</span> : null}
      </div>
    );
  }
  if (field.type === "enum") {
    return (
      <div className="stack" style={{ gap: 4 }}>
        <select className="select" value={String(value ?? "")} disabled={!field.editable} onChange={(event) => onChange(event.target.value)}>
          {field.options.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
        </select>
        {error ? <span className="field-error">{error}</span> : null}
      </div>
    );
  }
  return (
    <div className="stack" style={{ gap: 4 }}>
      <input className="input" value={String(value ?? "")} disabled={!field.editable} onChange={(event) => onChange(event.target.value)} />
      {error ? <span className="field-error">{error}</span> : null}
    </div>
  );
}

function DiffModal({ diff, onClose, onApply }: { diff: Record<string, unknown>; onClose: () => void; onApply: () => void }) {
  const changes = (diff.changes ?? []) as { key: string; label: string; oldValue: unknown; newValue: unknown; effect: string; risk: string }[];
  const restart = (diff.restartRequired ?? []) as string[];
  return (
    <Modal title="应用前差异" sub={`${changes.length} 项变更 · ${restart.length} 项需要重启`} onClose={onClose} wide footer={<><button type="button" className="btn" onClick={onClose}>取消</button><button type="button" className="btn btn-primary" onClick={onApply} disabled={changes.length === 0}><IconCheck size={13} /> 确认应用</button></>}>
      <div className="stack" style={{ gap: 10 }}>
        {changes.length === 0 ? <Empty title="没有实际变更" /> : changes.map((change) => (
          <div className="step" key={change.key}>
            <span className={`step-index`} style={{ background: change.risk === "high" ? "var(--warn-soft)" : undefined, color: change.risk === "high" ? "var(--warn)" : undefined }}>Δ</span>
            <div className="stack" style={{ gap: 3 }}>
              <span className="step-title">{change.label}</span>
              <span className="step-meta mono">{change.key} · {change.effect} · {change.risk}</span>
              <span className="small"><span className="muted">旧</span> {formatValue(change.oldValue)} → <span className="muted">新</span> {formatValue(change.newValue)}</span>
            </div>
          </div>
        ))}
        {restart.length ? <div className="alert warn">需要重启的字段：{restart.join("、")}。应用后 generation 已创建，但列出的项在 collector 重启后生效。</div> : null}
      </div>
    </Modal>
  );
}

function formatValue(value: unknown): string {
  if (typeof value === "string") return value || "（空）";
  return JSON.stringify(value);
}

function ModelModal({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const { notify } = useUi();
  const [name, setName] = useState("");
  const [kind, setKind] = useState("remoteApi");
  const [endpoint, setEndpoint] = useState("https://api.openai.com/v1");
  const [secretRef, setSecretRef] = useState("OPENAI_API_KEY");
  const [model, setModel] = useState("gpt-4.1-mini");
  const [enabled, setEnabled] = useState(true);
  const [busy, setBusy] = useState(false);
  return (
    <Modal
      title="登记模型"
      sub="密钥只登记环境变量名；值不会出现在接口、日志或差异里"
      onClose={onClose}
      footer={<><button type="button" className="btn" onClick={onClose}>取消</button><button type="button" className="btn btn-primary" disabled={busy || !name.trim() || !endpoint.trim()} onClick={() => void (async () => {
        setBusy(true);
        try {
          await createModel({ name, kind, endpoint, secretRef: secretRef || null, enabled, model, timeoutMs: 90_000, maxTokens: 1200, temperature: 0.3 });
          notify("ok", "模型已登记");
          onDone();
        } catch (problem) {
          notify("bad", "登记模型失败", problem instanceof Error ? problem.message : undefined);
        } finally {
          setBusy(false);
        }
      })()}>{busy ? "保存中…" : "登记"}</button></>}
    >
      <div className="stack" style={{ gap: 12 }}>
        <div className="grid cols-2">
          <Field label="名称"><input className="input" value={name} onChange={(event) => setName(event.target.value)} /></Field>
          <Field label="类型">
            <select className="select" value={kind} onChange={(event) => setKind(event.target.value)}>
              <option value="remoteApi">远程 API</option>
              <option value="localService">本机服务</option>
              <option value="localCli">本机 CLI</option>
            </select>
          </Field>
        </div>
        <Field label="端点"><input className="input mono" value={endpoint} onChange={(event) => setEndpoint(event.target.value)} /></Field>
        <Field label="模型名"><input className="input mono" value={model} onChange={(event) => setModel(event.target.value)} /></Field>
        <Field label="密钥环境变量"><input className="input mono" value={secretRef} onChange={(event) => setSecretRef(event.target.value)} placeholder="留空表示无密钥" /></Field>
        <label className="switch"><input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} /><span>启用模型</span></label>
      </div>
    </Modal>
  );
}

function NamedModal({ kind, onClose, onDone }: { kind: "prompt" | "skill"; onClose: () => void; onDone: () => void }) {
  const { notify } = useUi();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [content, setContent] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <Modal
      title={kind === "prompt" ? "新建提示词" : "新建 Skill"}
      onClose={onClose}
      wide
      footer={<><button type="button" className="btn" onClick={onClose}>取消</button><button type="button" className="btn btn-primary" disabled={busy || !name.trim() || !content.trim()} onClick={() => void (async () => {
        setBusy(true);
        try {
          if (kind === "prompt") await createPrompt({ name, description, content });
          else await createSkill({ name, description, content });
          notify("ok", "已创建");
          onDone();
        } catch (problem) {
          notify("bad", "创建失败", problem instanceof Error ? problem.message : undefined);
        } finally {
          setBusy(false);
        }
      })()}>{busy ? "保存中…" : "创建"}</button></>}
    >
      <div className="stack" style={{ gap: 12 }}>
        <Field label="名称"><input className="input" value={name} onChange={(event) => setName(event.target.value)} /></Field>
        <Field label="说明"><input className="input" value={description} onChange={(event) => setDescription(event.target.value)} /></Field>
        <Field label="正文"><textarea className="textarea" style={{ minHeight: 240 }} value={content} onChange={(event) => setContent(event.target.value)} /></Field>
      </div>
    </Modal>
  );
}

function VersionModal({ resource, onClose, onDone }: { resource: PromptResource; onClose: () => void; onDone: () => void }) {
  const { notify } = useUi();
  const [content, setContent] = useState(resource.versions[0]?.content ?? "");
  const [busy, setBusy] = useState(false);
  const isSkill = resource.id.startsWith("skill");
  return (
    <Modal
      title={`新版本 · ${resource.name}`}
      sub={`当前 ${resource.latestVersionId}；新版本只追加`}
      onClose={onClose}
      wide
      footer={<><button type="button" className="btn" onClick={onClose}>取消</button><button type="button" className="btn btn-primary" disabled={busy || !content.trim()} onClick={() => void (async () => {
        setBusy(true);
        try {
          if (isSkill) await createSkillVersion(resource.id, resource.etag ?? "", { content });
          else await createPromptVersion(resource.id, resource.etag ?? "", { content });
          notify("ok", "新版本已创建");
          onDone();
        } catch (problem) {
          notify("bad", "创建版本失败", problem instanceof Error ? problem.message : undefined);
        } finally {
          setBusy(false);
        }
      })()}>{busy ? "保存中…" : "创建版本"}</button></>}
    >
      <textarea className="textarea" style={{ minHeight: 320 }} value={content} onChange={(event) => setContent(event.target.value)} />
    </Modal>
  );
}

function MaintenanceCard({ busy, onPreview, onFactory, reportId }: { busy: boolean; onPreview: (domains: string[]) => void; onFactory: () => void; reportId: string | null }) {
  const [domains, setDomains] = useState<string[]>([]);
  return (
    <div className="stack" style={{ gap: 16 }}>
      <Card title="分域清空" sub="预演只统计数量；确认后永久删除所选域的元数据与对象">
        <div className="stack" style={{ gap: 12 }}>
          <div className="domain-grid">
            {DOMAINS.map(([id, label, note]) => (
              <label className="domain-option" key={id} title={note}>
                <input type="checkbox" checked={domains.includes(id)} onChange={(event) => setDomains((current) => event.target.checked ? [...current, id] : current.filter((item) => item !== id))} />
                <span>{label}</span>
              </label>
            ))}
          </div>
          <div className="row">
            <button type="button" className="btn btn-primary" disabled={busy || domains.length === 0} onClick={() => onPreview(domains)}>预演清空</button>
            <span className="small muted">选择内容会自动加入派生数据、生产输入与无引用对象。预演后需要在弹窗中再次确认才会真实删除。</span>
          </div>
        </div>
      </Card>
      <div className="danger-zone">
        <h3>恢复出厂设置</h3>
        <p className="small">
          覆盖元数据库、对象与暂存、派生索引、外部对象以及用户信源、模型、提示词、Skill 与设置历史；环境变量与其中的密钥逐字节保留。
        </p>
        <div className="row">
          <button type="button" className="btn btn-danger" disabled={busy} onClick={onFactory}>预演恢复出厂</button>
          {reportId ? <span className="small muted">最近报告 <code>{reportId}</code></span> : null}
        </div>
      </div>
    </div>
  );
}

function ResetModal({ preview, factory, onClose, onConfirm }: { preview: ResetPreview; factory: boolean; onClose: () => void; onConfirm: () => void }) {
  return (
    <Modal
      title={factory ? "确认恢复出厂" : "确认清空"}
      sub="执行需要预演令牌、ETag 与范围一致"
      onClose={onClose}
      wide
      footer={<><button type="button" className="btn" onClick={onClose}>取消</button><button type="button" className="btn btn-danger" onClick={onConfirm}>{factory ? "恢复出厂" : "执行清空"}</button></>}
    >
      <div className="stack" style={{ gap: 12 }}>
        <dl className="kv">
          <dt>范围</dt><dd>{preview.domains.join("、")}</dd>
          <dt>令牌有效期</dt><dd>{stamp(preview.expiresAt)}</dd>
          <dt>存储可达</dt><dd>{preview.storageReachable ? "是" : "否"}</dd>
        </dl>
        {preview.dependencies.length ? <div className="alert warn">自动加入：{preview.dependencies.map((item) => `${item.domain}（${item.reason}）`).join("；")}</div> : null}
        <div className="table-wrap">
          <table className="table">
            <thead><tr><th>域</th><th>数量</th><th>字节</th></tr></thead>
            <tbody>
              {Object.entries(preview.counts).map(([domain, value]) => (
                <tr key={domain}><td>{domain}</td><td className="mono">{value.count}</td><td className="mono">{value.bytes}</td></tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="alert bad">这是不可逆操作。确认令牌只在本页有效；关闭后需要重新预演。执行完成后会核对业务域是否归零。</div>
      </div>
    </Modal>
  );
}
