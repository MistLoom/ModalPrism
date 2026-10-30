import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { IconCheck, IconPlus, IconRefresh, IconTrash } from "../components/icons";
import { RowSubject } from "../components/ListRow";
import { PlatformIcon } from "../components/platforms";
import { Card, Empty, Field, Loading, Modal, Problem, StatusBadge, inlineValue, useUi } from "../components/ui";
import {
  applySettings,
  createFactoryReset,
  createModel,
  createPrompt,
  createPromptVersion,
  createReset,
  createSkill,
  createSkillVersion,
  deleteModel,
  diffSettings,
  discoverClis,
  discoverModels,
  getEffectiveSettings,
  getSettingsSchema,
  listModels,
  listPrompts,
  listSearchPlatforms,
  listSkills,
  previewFactoryReset,
  previewReset,
  rollbackSettings,
  saveSearchPlatforms,
  settingsHistory,
  testModel,
  testSearchEngine,
  updateModel,
  validateSettings,
} from "../lib/api";
import { stamp } from "../lib/format";
import { CUSTOM_PRESET, MODEL_PRESETS, profileSummary, resolveProfile } from "../lib/model-presets";
import type {
  CliCandidate,
  EffectiveSettings,
  ModelResource,
  Operation,
  PromptResource,
  ResetPreview,
  SearchPlatform,
  SettingsField,
  SettingsGroup,
  SettingsHistoryEntry,
  SettingsSchema,
} from "../lib/types";

type Section = string;

const DOMAINS = [
  ["tasks", "任务", "排队、活动与终态任务"],
  ["content", "内容", "版本、关联与历史记录"],
  ["blobs", "对象", "无引用的本地或外部对象"],
  ["derivedData", "派生数据", "全文、向量等派生索引"],
  ["delivery", "投递", "投递规则、记录、渠道与通知"],
  ["staging", "暂存", "未提交对象与过期上传"],
  ["logs", "日志", "普通日志与结构化事件"],
  ["audit", "审计", "不可修改的治理事件"],
  ["operations", "操作", "已终结的 Operation"],
  ["userSettings", "用户设置", "模型、提示词与技能"],
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
  const [modelEditing, setModelEditing] = useState<ModelResource | null>(null);
  const [rowTesting, setRowTesting] = useState<string | null>(null);
  const [promptOpen, setPromptOpen] = useState<"prompt" | "skill" | null>(null);
  const [versionFor, setVersionFor] = useState<PromptResource | null>(null);
  const [preview, setPreview] = useState<ResetPreview | null>(null);
  const [factory, setFactory] = useState(false);
  const [reportId, setReportId] = useState<string | null>(null);
  // 已提交但未结束的清空/恢复出厂操作：用于危险区持续提示，避免“点了没反应”。
  const [resetOp, setResetOp] = useState<Operation | null>(null);
  // 提交锁：状态更新是异步的，连点会发出两次 POST（第二次 409），用 ref 兑住。
  const submitting = useRef(false);
  const [modelSelection, setModelSelection] = useState<string[]>([]);

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
    if (!preview || busy || submitting.current) return;
    submitting.current = true;
    setBusy(true);
    let started = false;
    const label = factory ? "恢复出厂设置" : "分域清空";
    try {
      const operation = factory
        ? await createFactoryReset(preview, preview.confirmationToken)
        : await createReset(preview, preview.confirmationToken);
      started = true;
      // 202 即已受理：立刻关掉确认弹窗并给出反馈，随后才等终态。
      // 否则弹窗会一直盖住左下角的操作进度卡片，看起来像“点了没反应”。
      setPreview(null);
      setFactory(false);
      setResetOp(operation);
      notify("ok", `${label}已提交`, "正在排空任务并清理数据，进度见左下角任务卡片。");
      const finished = await run(operation, label);
      const summary = (finished.resultSummary ?? {}) as Record<string, unknown>;
      setReportId(String(summary.reportId ?? "") || null);
      await load();
    } catch (problem) {
      // 提交前的失败（令牌/ETag 不一致）需要提示；提交后的失败 run 已提示过。
      if (!started) {
        notify("bad", "执行失败", problem instanceof Error ? problem.message : undefined);
      }
    } finally {
      submitting.current = false;
      setResetOp(null);
      setBusy(false);
    }
  }

  // 模型批量操作：逐项执行、保留成功项；被引用的模型删除会失败并汇总提示。
  async function batchModels(action: "enable" | "disable" | "delete") {
    const targets = models.filter((model) => modelSelection.includes(model.id));
    if (!targets.length) return;
    setBusy(true);
    let succeeded = 0;
    let failed = 0;
    for (const model of targets) {
      try {
        if (action === "delete") await deleteModel(model.id);
        else await updateModel(model.id, { enabled: action === "enable" });
        succeeded += 1;
      } catch {
        failed += 1;
      }
    }
    const label = action === "enable" ? "批量启用" : action === "disable" ? "批量停用" : "批量删除";
    notify(failed ? "warn" : "ok", `${label}：成功 ${succeeded} 个`, failed ? `失败 ${failed} 个（可能被默认模型或工作流版本引用）。` : undefined);
    setModelSelection([]);
    await load();
    setBusy(false);
  }

  if (loading && !schema) {
    return <div className="page"><Loading lines={6} /></div>;
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <div className="kicker">settings · 系统设置</div>
          <h1 className="page-title">系统设置</h1>
          <p className="page-lede">
            设置展示当前生效值，应用后生成新版本。模型、提示词与 Skill 为版本化资源，生成物需批准后启用。
          </p>
        </div>
        <div className="page-actions">
          <StatusBadge status={effective ? "online" : "degraded"} />
          <span className="small muted">当前版本 <strong className="mono">G-{effective?.generation ?? "—"}</strong></span>
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
          <button type="button" className={section === "search" ? "is-active" : ""} onClick={() => setSection("search")}>搜索平台</button>
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
            <>
              {modelSelection.length > 0 ? (
                <div className="batchbar">
                  <span>已选 {modelSelection.length} 个模型</span>
                  <button type="button" className="btn btn-sm" disabled={busy} onClick={() => void batchModels("enable")}>批量启用</button>
                  <button type="button" className="btn btn-sm" disabled={busy} onClick={() => void batchModels("disable")}>批量停用</button>
                  <button
                    type="button"
                    className="btn btn-sm btn-danger"
                    disabled={busy}
                    onClick={() => {
                      if (!window.confirm(`删除选中的 ${modelSelection.length} 个模型？被默认模型或工作流引用的模型将删除失败。`)) return;
                      void batchModels("delete");
                    }}
                  >
                    <IconTrash size={12} /> 批量删除
                  </button>
                  <button type="button" className="btn btn-sm" onClick={() => setModelSelection([])}>清除选择</button>
                </div>
              ) : null}
              <Card title="模型资源" sub="密钥支持环境变量引用或直接填写（仅写入，不回读）；被版本引用的模型不可删除" action={<button type="button" className="btn btn-sm" onClick={() => setModelOpen(true)}><IconPlus size={12} /> 登记模型</button>} flush>
                {models.length === 0 ? <Empty title="暂无模型" text="登记 OpenAI 兼容端点、本机服务或命令行模型。" /> : (
                  <div className="list">
                    <div className="list-row" style={{ borderBottom: "1px solid var(--line-soft)" }}>
                      <input
                        type="checkbox"
                        aria-label="全选模型"
                        checked={models.length > 0 && modelSelection.length === models.length}
                        onChange={(event) => setModelSelection(event.target.checked ? models.map((model) => model.id) : [])}
                      />
                      <span className="list-main"><span className="list-title small muted">全选（{models.length} 个模型）</span></span>
                    </div>
                    {models.map((model) => (
                      <div className={`list-row${modelSelection.includes(model.id) ? " is-selected" : ""}`} key={model.id}>
                        <input
                          type="checkbox"
                          aria-label={`选择 ${model.name}`}
                          checked={modelSelection.includes(model.id)}
                          onChange={(event) =>
                            setModelSelection((current) =>
                              event.target.checked ? [...current, model.id] : current.filter((id) => id !== model.id),
                            )
                          }
                        />
                        <span className="list-main">
                          <RowSubject onClick={() => setModelEditing(model)} title="编辑模型">
                            {model.name}
                          </RowSubject>
                          <span className="list-meta">
                            <span className="mono">{model.id}</span>
                            <span>{model.kind}</span>
                            <span>{model.model}</span>
                            <span>{model.secretInline ? "内置密钥" : model.secretRef ? `密钥 ${model.secretRef}${model.secretPresent ? "（存在）" : "（缺失）"}` : "无密钥"}</span>
                            <span>{modelCapabilityText(model)}</span>
                          </span>
                        </span>
                        <span className={`badge ${model.enabled ? "ok" : "neutral"}`}>{model.enabled ? "启用" : "停用"}</span>
                        <button
                          type="button"
                          className="btn btn-sm"
                          disabled={busy || rowTesting === model.id}
                          onClick={() => void (async () => {
                            setRowTesting(model.id);
                            try {
                              const result = await testModel({ id: model.id });
                              notify("ok", "连通正常", result.mode === "cli" ? `${result.latencyMs}ms · ${result.version ?? ""}` : `${result.latencyMs}ms · ${result.sample ?? ""}`);
                            } catch (problem) {
                              notify("bad", "连通失败", problem instanceof Error ? problem.message : undefined);
                            } finally {
                              setRowTesting(null);
                            }
                          })()}
                        >
                          {rowTesting === model.id ? "测试中…" : "测试"}
                        </button>
                      </div>
                    ))}
                  </div>
                )}
              </Card>
            </>
          ) : null}

          {section === "search" ? <SearchPlatforms /> : null}

          {section === "prompts" || section === "skills" ? (
            <Card
              title={section === "prompts" ? "提示词" : "Skill"}
              sub="标识固定、按版本管理；被运行引用的版本不可修改"
              action={<button type="button" className="btn btn-sm" onClick={() => setPromptOpen(section === "prompts" ? "prompt" : "skill")}><IconPlus size={12} /> 新建</button>}
              flush
            >
              {(section === "prompts" ? prompts : skills).length === 0 ? <Empty title="暂无内容" /> : (
                <div className="list">
                  {(section === "prompts" ? prompts : skills).map((item) => (
                    <div className="list-row" key={item.id}>
                      <span className="list-main">
                        <RowSubject onClick={() => setVersionFor(item)} title="查看版本并新建">
                          {item.name}{item.builtin ? <span className="tag" style={{ marginLeft: 6 }}>内置</span> : null}
                        </RowSubject>
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
            <Card title="变更历史" sub="回滚生成新版本，保留历史" flush>
              {history.length === 0 ? <Empty title="暂无历史" /> : (
                <div className="table-wrap">
                  <table className="table">
                    <thead><tr><th>版本</th><th>摘要</th><th>应用时间</th><th>回滚来源</th><th /></tr></thead>
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
              pending={resetOp}
              onPreview={(domains) => void previewResetDomains(domains)}
              onFactory={() => void previewFactory()}
              reportId={reportId}
            />
          ) : null}
        </div>
      </div>

      {diff ? <DiffModal diff={diff} onClose={() => setDiff(null)} onApply={() => { setDiff(null); void apply(); }} /> : null}
      {modelOpen || modelEditing ? (
        <ModelModal
          initial={modelEditing ?? undefined}
          onClose={() => { setModelOpen(false); setModelEditing(null); }}
          onDone={() => { setModelOpen(false); setModelEditing(null); void load(); }}
        />
      ) : null}
      {promptOpen ? <NamedModal kind={promptOpen} onClose={() => setPromptOpen(null)} onDone={() => { setPromptOpen(null); void load(); }} /> : null}
      {versionFor ? <VersionModal resource={versionFor} onClose={() => setVersionFor(null)} onDone={() => { setVersionFor(null); void load(); }} /> : null}
      {preview ? (
        <ResetModal
          preview={preview}
          factory={factory}
          busy={busy}
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
        {restart.length ? <div className="alert warn">需要重启的字段：{restart.join("、")}。应用后已生成新版本，所列字段在 collector 重启后生效。</div> : null}
      </div>
    </Modal>
  );
}

function formatValue(value: unknown): string {
  return inlineValue(value);
}

type SecretMode = "none" | "env" | "inline";

/** 能力摘要：优先用已存储的声明，缺失时按预设解析，避免旧模型显示为空。 */
function modelCapabilityText(model: ModelResource): string {
  const fallback = resolveProfile(model.model ?? model.name ?? "");
  return profileSummary({
    contextWindow: model.contextWindow ?? fallback.contextWindow,
    maxTokens: model.maxTokens ?? fallback.maxTokens,
    reasoning: model.reasoning ?? fallback.reasoning,
    modalities: model.modalities?.length ? model.modalities : fallback.modalities,
    temperature: model.temperature === undefined ? fallback.temperature : model.temperature,
  });
}

/// 登记或编辑模型。密钥三态：无 / 环境变量引用 / 直接填写（值只写不读，列表与差异不出现）。
/// localCli 支持扫描本机常见 agent CLI；保存前可对当前表单做连通测试。
function ModelModal({ initial, onClose, onDone }: { initial?: ModelResource; onClose: () => void; onDone: () => void }) {
  const { notify } = useUi();
  const editing = Boolean(initial);
  // 初始值跟随第一个预设，避免与 model-presets.ts 双处维护漂移。
  const [preset, setPreset] = useState(MODEL_PRESETS[0].id);
  const [name, setName] = useState(initial?.name ?? "");
  const [kind, setKind] = useState<string>(initial?.kind ?? MODEL_PRESETS[0].kind);
  const [endpoint, setEndpoint] = useState(initial?.endpoint ?? MODEL_PRESETS[0].endpoint);
  const [model, setModel] = useState(initial?.model ?? MODEL_PRESETS[0].models[0] ?? "");
  const initialProfile = resolveProfile(initial?.model ?? MODEL_PRESETS[0].models[0] ?? "");
  // 能力缺省：新建时随模型名自动带出；手工改过就不再覆盖。
  const [contextWindow, setContextWindow] = useState(String(initial?.contextWindow ?? initialProfile.contextWindow));
  const [maxTokens, setMaxTokens] = useState(String(initial?.maxTokens ?? initialProfile.maxTokens));
  // 温度留空 = 按官方缺省、不向请求注入 temperature（思考型模型常见要求）。
  const initialTemperature = initial
    ? (typeof initial.temperature === "number" ? String(initial.temperature) : "")
    : (initialProfile.temperature === null ? "" : String(initialProfile.temperature));
  const [temperature, setTemperature] = useState(initialTemperature);
  const [reasoning, setReasoning] = useState(initial?.reasoning ?? initialProfile.reasoning);
  const [modalities, setModalities] = useState<string[]>(initial?.modalities?.length ? initial.modalities : initialProfile.modalities);
  const [capTouched, setCapTouched] = useState(Boolean(initial));
  const profile = {
    contextWindow: Number(contextWindow) || 0,
    maxTokens: Number(maxTokens) || 0,
    reasoning,
    modalities,
    temperature: temperature.trim() ? Number(temperature) : null,
  };
  const [enabled, setEnabled] = useState(initial?.enabled ?? true);
  const [secretMode, setSecretMode] = useState<SecretMode>(
    initial ? (initial.secretInline ? "inline" : initial.secretRef ? "env" : "none")
      : MODEL_PRESETS[0].secretRef ? "env" : "none",
  );
  const [secretRef, setSecretRef] = useState(initial?.secretRef ?? MODEL_PRESETS[0].secretRef);
  const [secretValue, setSecretValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; text: string } | null>(null);
  const [clis, setClis] = useState<CliCandidate[] | null>(null);
  const [scanning, setScanning] = useState(false);
  // 平台模型清单：按当前端点与密钥实时拉取，预设只是离线兜底。
  const [remoteModels, setRemoteModels] = useState<string[]>([]);
  const [discovering, setDiscovering] = useState(false);
  const presetModels = MODEL_PRESETS.find((item) => item.id === preset)?.models ?? [];
  const candidates = useMemo(() => {
    const seen = new Set<string>();
    const merged: string[] = [];
    for (const item of [...presetModels, ...remoteModels]) {
      if (!seen.has(item)) {
        seen.add(item);
        merged.push(item);
      }
    }
    return merged;
  }, [presetModels, remoteModels]);
  // 选平台即填充类型、端点、密钥环境变量名与模型候选；全部字段仍可手改。
  function applyPreset(id: string) {
    setPreset(id);
    const found = [...MODEL_PRESETS, CUSTOM_PRESET].find((item) => item.id === id);
    if (!found) return;
    setKind(found.kind);
    if (found.endpoint) setEndpoint(found.endpoint);
    setSecretRef(found.secretRef);
    // 用户已选"直接填写"时保留其密钥模式，只更新环境变量名备用。
    if (secretMode !== "inline") setSecretMode(found.secretRef ? "env" : "none");
    setModel(found.models[0] ?? "");
    setRemoteModels([]);
  }
  // 表单字段变化后旧的连通结论立即失效。
  useEffect(() => {
    setTestResult(null);
  }, [kind, endpoint, model, secretMode, secretRef, secretValue]);
  // 模型名变化时按预设刷新能力缺省；已经手改过则保留用户的选择。
  useEffect(() => {
    if (capTouched) return;
    const next = resolveProfile(model);
    setContextWindow(String(next.contextWindow));
    setMaxTokens(String(next.maxTokens));
    setReasoning(next.reasoning);
    setModalities(next.modalities);
    setTemperature(next.temperature === null ? "" : String(next.temperature));
  }, [model, capTouched]);
  function markCapability() {
    setCapTouched(true);
  }
  // 编辑已登记模型时也能拉取最新预设：型号规格随官方文档调整，旧声明需要可重置。
  function applyPresetCapability() {
    const next = resolveProfile(model);
    setContextWindow(String(next.contextWindow));
    setMaxTokens(String(next.maxTokens));
    setReasoning(next.reasoning);
    setModalities(next.modalities);
    setTemperature(next.temperature === null ? "" : String(next.temperature));
    setCapTouched(false);
  }
  function toggleModality(name: string) {
    markCapability();
    setModalities((current) => {
      if (!current.includes(name)) return [...current, name];
      // 文本是必选模态，不允许清空到无输入。
      if (current.length === 1) return current;
      return current.filter((item) => item !== name);
    });
  }
  // 当前表单的密钥字段组：返回 null 表示沿用已存储的内置密钥（仅编辑场景）。
  function secretFields(): { secretRef?: string | null; secretValue?: string | null } | null {
    if (secretMode === "env") return { secretRef: secretRef.trim() || null, secretValue: null };
    if (secretMode === "inline") {
      if (editing && !secretValue.trim() && initial?.secretInline) return null;
      return { secretRef: null, secretValue: secretValue.trim() || null };
    }
    return { secretRef: null, secretValue: null };
  }
  async function test() {
    setTesting(true);
    setTestResult(null);
    try {
      const fields = secretFields();
      const input: Record<string, unknown> = { kind, endpoint, model };
      if (fields) Object.assign(input, fields);
      else input.id = initial?.id; // 沿用存储密钥：按 id 测，端点/模型名用表单覆盖。
      const result = await testModel(input);
      setTestResult({
        ok: true,
        text: result.mode === "cli"
          ? `命令可执行（${result.latencyMs}ms）：${result.version ?? ""}`
          : `补全成功（${result.latencyMs}ms）：${result.sample ?? ""}`,
      });
    } catch (problem) {
      setTestResult({ ok: false, text: problem instanceof Error ? problem.message : "连通测试失败" });
    } finally {
      setTesting(false);
    }
  }
  async function scan() {
    setScanning(true);
    try {
      const page = await discoverClis();
      setClis(page.items);
      if (!page.items.some((item) => item.found)) {
        notify("warn", "未发现已安装的 agent CLI", "候选列表仍可手动填写命令。");
      }
    } catch (problem) {
      notify("bad", "扫描失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setScanning(false);
    }
  }
  async function pullModels() {
    if (!endpoint.trim()) {
      notify("bad", "无法拉取", "先填写端点地址。");
      return;
    }
    setDiscovering(true);
    try {
      const page = await discoverModels({
        kind,
        endpoint: endpoint.trim(),
        secretRef: secretMode === "env" ? secretRef || null : null,
        secretValue: secretMode === "inline" ? secretValue || null : null,
      });
      setRemoteModels(page.items);
      if (page.items.length === 0) {
        notify("warn", "平台没有返回模型", "确认端点是否正确、密钥是否有列举权限。");
      } else {
        notify("ok", "已拉取平台模型", `${page.items.length} 个候选，可在模型名下拉中选择`);
        if (!model.trim()) setModel(page.items[0]);
      }
    } catch (problem) {
      notify("bad", "拉取模型失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setDiscovering(false);
    }
  }
  return (
    <Modal
      title={editing ? `编辑模型 · ${initial?.name ?? ""}` : "登记模型"}
      sub="密钥支持环境变量引用或直接填写；直接填写的值仅写入，不出现在列表、日志与差异中"
      onClose={onClose}
      wide
      footer={<>
        <button type="button" className="btn" onClick={onClose}>取消</button>
        <button type="button" className="btn" disabled={testing || busy || !endpoint.trim()} onClick={() => void test()}>
          {testing ? "测试中…" : "连通测试"}
        </button>
        <button type="button" className="btn btn-primary" disabled={busy || testing || !name.trim() || !endpoint.trim()} onClick={() => void (async () => {
          setBusy(true);
          try {
            const fields = secretFields();
            const capability = {
              contextWindow: Number(contextWindow) || undefined,
              maxTokens: Number(maxTokens) || undefined,
              reasoning,
              modalities,
              temperature: temperature.trim() ? Number(temperature) : null,
            };
            if (editing && initial) {
              await updateModel(initial.id, { name, endpoint, model, enabled, ...capability, ...(fields ?? {}) });
              notify("ok", "模型已更新");
            } else {
              await createModel({ name, kind, endpoint, ...(fields ?? {}), enabled, model, timeoutMs: 90_000, ...capability });
              notify("ok", "模型已登记");
            }
            onDone();
          } catch (problem) {
            notify("bad", editing ? "更新模型失败" : "登记模型失败", problem instanceof Error ? problem.message : undefined);
          } finally {
            setBusy(false);
          }
        })()}>{busy ? "保存中…" : editing ? "保存" : "登记"}</button>
      </>}
    >
      <div className="stack" style={{ gap: 12 }}>
        {!editing ? (
          <Field label="快速配置" hint="选择常见平台自动填入端点与密钥环境变量，字段仍可手改">
            <select className="select" value={preset} onChange={(event) => applyPreset(event.target.value)}>
              {MODEL_PRESETS.map((item) => (
                <option key={item.id} value={item.id}>{item.label}</option>
              ))}
              <option value={CUSTOM_PRESET.id}>{CUSTOM_PRESET.label}</option>
            </select>
          </Field>
        ) : null}
        <div className="grid cols-2">
          <Field label="名称"><input className="input" value={name} onChange={(event) => setName(event.target.value)} /></Field>
          <Field label="类型" hint={editing ? "类型不可变更；换类型请重新登记" : undefined}>
            <select
              className="select"
              value={kind}
              disabled={editing}
              onChange={(event) => {
                const next = event.target.value;
                // 切到 CLI 时端点（URL）语义失效：清空避免串进命令字段。
                if (next === "localCli" && kind !== "localCli") setEndpoint("");
                setKind(next);
              }}
            >
              <option value="remoteApi">远程 API</option>
              <option value="localService">本机服务</option>
              <option value="localCli">本机 CLI</option>
            </select>
          </Field>
        </div>
        {kind === "localCli" ? (
          <Field label="命令" hint="CLI 的调用命令（含参数），提示词经标准输入送入">
            <div className="row" style={{ gap: 8 }}>
              <input className="input mono" style={{ flex: 1 }} value={endpoint} onChange={(event) => setEndpoint(event.target.value)} />
              <button type="button" className="btn btn-sm" disabled={scanning} onClick={() => void scan()}>
                <IconRefresh size={12} /> {scanning ? "扫描中…" : "扫描发现"}
              </button>
            </div>
            {clis ? (
              <div className="cli-list">
                {clis.map((item) => (
                  <button
                    type="button"
                    key={item.id}
                    className={`cli-item${item.found ? "" : " is-missing"}`}
                    disabled={!item.found}
                    title={item.version}
                    onClick={() => { setEndpoint(item.command); if (!name.trim()) setName(item.label); setPreset(CUSTOM_PRESET.id); }}
                  >
                    <span className={`dot ${item.found ? "ok" : ""}`} />
                    <span className="cli-name">{item.label}</span>
                    <span className="mono small">{item.command}</span>
                    <span className="small muted">{item.found ? item.version : "未发现"}</span>
                  </button>
                ))}
              </div>
            ) : null}
          </Field>
        ) : (
          <Field label="端点" hint="OpenAI 兼容基地址；调用时自动补 /chat/completions">
            <input className="input mono" value={endpoint} onChange={(event) => setEndpoint(event.target.value)} />
          </Field>
        )}
        <Field
          label="模型名"
          hint={`可直接输入，也可从平台实时拉取清单（当前候选 ${candidates.length} 个）`}
        >
          <div className="row" style={{ gap: 8 }}>
            <input className="input mono" list="model-preset-options" value={model} onChange={(event) => setModel(event.target.value)} />
            <button type="button" className="btn btn-sm" disabled={discovering || !endpoint.trim()} onClick={() => void pullModels()}>
              {discovering ? "拉取中…" : "拉取平台模型"}
            </button>
          </div>
          <datalist id="model-preset-options">
            {candidates.map((item) => <option key={item} value={item} />)}
          </datalist>
        </Field>
        <Field label="能力缺省" hint="随模型名自动带出（上下文、输出上限、思考模式、输入模态）；可手改，登记后作为该模型的预算与能力声明">
          <div className="stack" style={{ gap: 8 }}>
            <div className="grid cols-2">
              <label className="row" style={{ gap: 6 }}>
                <span className="small muted" style={{ width: 72 }}>上下文</span>
                <input
                  className="input mono"
                  type="number"
                  min={1024}
                  step={1024}
                  value={contextWindow}
                  onChange={(event) => { markCapability(); setContextWindow(event.target.value); }}
                />
                <span className="small muted">tokens</span>
              </label>
              <label className="row" style={{ gap: 6 }}>
                <span className="small muted" style={{ width: 72 }}>输出上限</span>
                <input
                  className="input mono"
                  type="number"
                  min={64}
                  step={64}
                  value={maxTokens}
                  onChange={(event) => { markCapability(); setMaxTokens(event.target.value); }}
                />
                <span className="small muted">tokens</span>
              </label>
            </div>
            <div className="row wrap" style={{ gap: 12 }}>
              <label className="switch">
                <input type="checkbox" checked={reasoning} onChange={(event) => { markCapability(); setReasoning(event.target.checked); }} />
                <span>支持思考模式</span>
              </label>
              <div className="seg" role="group" aria-label="输入模态">
                {["text", "image", "audio", "video"].map((name) => (
                  <button
                    key={name}
                    type="button"
                    className={modalities.includes(name) ? "is-active" : ""}
                    aria-pressed={modalities.includes(name)}
                    onClick={() => toggleModality(name)}
                  >
                    {name}
                  </button>
                ))}
              </div>
            </div>
            <div className="row between" style={{ gap: 8 }}>
              <span className="small muted">
                {profileSummary(profile)} · {capTouched ? "已手改" : "来自预设"}
              </span>
              <button type="button" className="btn btn-ghost btn-sm" onClick={applyPresetCapability}>
                按预设重置
              </button>
            </div>
            <label className="row" style={{ gap: 6 }}>
              <span className="small muted" style={{ width: 72 }}>采样温度</span>
              <input
                className="input mono"
                type="number"
                min={0}
                max={2}
                step={0.1}
                value={temperature}
                placeholder="留空 = 官方缺省"
                onChange={(event) => { markCapability(); setTemperature(event.target.value); }}
              />
              <span className="small muted">留空则不发 temperature（思考型模型建议留空）</span>
            </label>
          </div>
        </Field>
        <Field label="密钥">
          <div className="stack" style={{ gap: 8 }}>
            <div className="seg" role="group" aria-label="密钥方式">
              {([["none", "无密钥"], ["env", "环境变量引用"], ["inline", "直接填写"]] as [SecretMode, string][]).map(([mode, label]) => (
                <button
                  key={mode}
                  type="button"
                  className={secretMode === mode ? "is-active" : ""}
                  aria-pressed={secretMode === mode}
                  onClick={() => setSecretMode(mode)}
                >
                  {label}
                </button>
              ))}
            </div>
            {secretMode === "env" ? (
              <input className="input mono" value={secretRef} onChange={(event) => setSecretRef(event.target.value)} placeholder="如 OPENAI_API_KEY" />
            ) : null}
            {secretMode === "inline" ? (
              <input
                className="input mono"
                type="password"
                autoComplete="off"
                value={secretValue}
                onChange={(event) => setSecretValue(event.target.value)}
                placeholder={editing && initial?.secretInline ? "已存有内置密钥；重新输入以替换" : "密钥值，保存后不可回读"}
              />
            ) : null}
          </div>
        </Field>
        <label className="switch"><input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} /><span>启用模型</span></label>
        {testResult ? (
          <div className={`test-result ${testResult.ok ? "ok" : "bad"}`}>
            {testResult.ok ? "连通正常" : "连通失败"}：{testResult.text}
          </div>
        ) : null}
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

function MaintenanceCard({
  busy,
  pending,
  onPreview,
  onFactory,
  reportId,
}: {
  busy: boolean;
  pending: Operation | null;
  onPreview: (domains: string[]) => void;
  onFactory: () => void;
  reportId: string | null;
}) {
  const [domains, setDomains] = useState<string[]>([]);
  return (
    <div className="stack" style={{ gap: 16 }}>
      {pending ? (
        <div className="alert warn">
          {pending.kind === "maintenance.factoryReset" ? "恢复出厂" : "清空"}已提交（
          <code>{pending.operationId}</code>），正在排空任务并清理数据。
          进度见左下角任务卡片，完成后本页会自动刷新。
        </div>
      ) : null}
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
            <button type="button" className="btn btn-primary" disabled={busy || pending !== null || domains.length === 0} onClick={() => onPreview(domains)}>预演清空</button>
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
          <button type="button" className="btn btn-danger" disabled={busy || pending !== null} onClick={onFactory}>
            {pending ? "执行中…" : "预演恢复出厂"}
          </button>
          {reportId ? <span className="small muted">最近报告 <code>{reportId}</code></span> : null}
        </div>
      </div>
    </div>
  );
}

function ResetModal({
  preview,
  factory,
  busy,
  onClose,
  onConfirm,
}: {
  preview: ResetPreview;
  factory: boolean;
  busy: boolean;
  onClose: () => void;
  onConfirm: () => void;
}) {
  return (
    <Modal
      title={factory ? "确认恢复出厂" : "确认清空"}
      sub="执行需要预演令牌、ETag 与范围一致"
      onClose={onClose}
      wide
      footer={
        <>
          <button type="button" className="btn" disabled={busy} onClick={onClose}>
            取消
          </button>
          <button type="button" className="btn btn-danger" disabled={busy} onClick={onConfirm}>
            {busy ? "提交中…" : factory ? "恢复出厂" : "执行清空"}
          </button>
        </>
      }
    >
      <div className="stack" style={{ gap: 12 }}>
        <dl className="kv">
          <dt>范围</dt><dd>{preview.domains.join("、")}</dd>
          <dt>令牌有效期</dt><dd>{stamp(preview.expiresAt)}</dd>
          <dt>存储可达</dt><dd>{preview.storageReachable ? "是" : "否"}</dd>
        </dl>
        {factory && preview.config ? (
          <div className={preview.config.baselineAvailable ? "alert" : "alert bad"}>
            {preview.config.baselineAvailable
              ? `配置将回到出厂基线，重建 ${preview.config.declarations} 份声明`
              : "出厂基线缺失或无法加载，恢复出厂会保留当前配置"}
            {preview.config.retiredPaths.length
              ? `；同时退役遗留目录 ${preview.config.retiredPaths.map((item) => item.path).join("、")}`
              : ""}
          </div>
        ) : null}
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
        <div className="alert bad">这是不可逆操作。确认令牌只在本页有效；关闭后需要重新预演。提交后弹窗会关闭，执行进度在左下角任务卡片中。</div>
      </div>
    </Modal>
  );
}

/** 搜索平台：与模型同一套卡片式快速配置（图标、多选启用、连通检测）。 */
function SearchPlatforms() {
  const { notify } = useUi();
  const [items, setItems] = useState<SearchPlatform[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, { ok: boolean; text: string }>>({});

  useEffect(() => {
    void (async () => {
      try {
        const page = await listSearchPlatforms();
        setItems(page.items);
      } catch (problem) {
        setError(problem);
      }
    })();
  }, []);

  function patch(engine: string, value: Partial<SearchPlatform>) {
    setItems((current) => (current ?? []).map((item) => (item.engine === engine ? { ...item, ...value } : item)));
  }

  async function save() {
    if (!items) return;
    setBusy(true);
    try {
      const page = await saveSearchPlatforms(items.map((item) => ({ engine: item.engine, enabled: item.enabled, keyRef: item.keyRef })));
      setItems(page.items);
      notify("ok", "搜索平台已保存");
    } catch (problem) {
      notify("bad", "保存搜索平台失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  async function test(item: SearchPlatform) {
    setTesting(item.engine);
    try {
      const result = await testSearchEngine({ engine: item.engine, keyRef: item.keyRef || undefined });
      setResults((current) => ({
        ...current,
        [item.engine]: { ok: true, text: `连通正常 · ${result.latencyMs}ms · 命中 ${result.count} 条` },
      }));
    } catch (problem) {
      setResults((current) => ({
        ...current,
        [item.engine]: { ok: false, text: problem instanceof Error ? problem.message : "连通失败" },
      }));
    } finally {
      setTesting(null);
    }
  }

  if (error) return <Problem error={error} />;
  if (!items) return <Loading lines={4} />;

  const enabled = items.filter((item) => item.enabled).length;

  return (
    <Card
      title="搜索平台"
      sub={`${enabled} / ${items.length} 个平台启用；密钥只保存环境变量引用名`}
      action={
        <button type="button" className="btn btn-primary btn-sm" disabled={busy} onClick={() => void save()}>
          {busy ? "保存中…" : "保存"}
        </button>
      }
    >
      <div className="platform-grid">
        {items.map((item) => (
          <div className={`platform-card static${item.enabled ? " is-on" : ""}`} key={item.engine}>
            <PlatformIcon icon={item.catalog.icon} label={item.catalog.name} size={34} />
            <div className="platform-card-main">
              <div className="platform-card-title">{item.catalog.name}</div>
              <div className="platform-card-note">{item.catalog.docs}</div>
              {item.catalog.fields.map((field) => (
                <input
                  key={field.key}
                  className="input mono input-sm"
                  value={item.keyRef}
                  placeholder={field.placeholder}
                  aria-label={`${item.catalog.name} ${field.label}`}
                  onChange={(event) => patch(item.engine, { keyRef: event.target.value })}
                />
              ))}
              {results[item.engine] ? (
                <div className={`test-result ${results[item.engine].ok ? "ok" : "bad"}`}>{results[item.engine].text}</div>
              ) : null}
            </div>
            <div className="row" style={{ gap: 6 }}>
              <button
                type="button"
                className="btn btn-sm"
                disabled={testing === item.engine}
                onClick={() => void test(item)}
              >
                {testing === item.engine ? "检测中…" : "连通检测"}
              </button>
              <label className="switch">
                <input type="checkbox" checked={item.enabled} onChange={(event) => patch(item.engine, { enabled: event.target.checked })} />
                <span>{item.enabled ? "启用" : "停用"}</span>
              </label>
            </div>
          </div>
        ))}
      </div>
    </Card>
  );
}
