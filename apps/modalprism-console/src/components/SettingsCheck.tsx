import { useCallback, useEffect, useRef, useState } from "react";

import { IconCheck, IconChevronDown, IconRefresh } from "./icons";
import { useUi } from "./ui";
import {
  applySettings,
  createModel,
  discoverModels,
  getEffectiveSettings,
  getPreconditions,
  listModels,
  systemStatus,
  validateSettings,
} from "../lib/api";
import { CUSTOM_PRESET, MODEL_PRESETS, profileSummary, resolveProfile } from "../lib/model-presets";
import { navigate } from "../lib/router";
import type { ModelResource, PreconditionReport, SystemStatus } from "../lib/types";

type Level = "ok" | "warn" | "bad" | "info";

interface CheckItem {
  id: string;
  label: string;
  level: Level;
  detail: string;
  action?: { label: string; run: () => void };
}

interface Snapshot {
  preconditions: PreconditionReport;
  models: ModelResource[];
  defaultModelId: string;
  etag: string;
  status: SystemStatus;
}

/// 设置就绪检查：模型/信源/搜索引擎/结构化解析是运行必须项。
/// 入口是一枚低干扰徽标，展开浮窗后未就绪项提供预设快速配置。
export function SettingsCheck() {
  const { notify, run } = useUi();
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const anchorRef = useRef<HTMLDivElement | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [preconditions, modelPage, effective, status] = await Promise.all([
        getPreconditions(),
        listModels(),
        getEffectiveSettings(),
        systemStatus(),
      ]);
      setSnap({
        preconditions,
        models: modelPage.items,
        defaultModelId: String(effective.data.values?.["ai.defaultModelId"] ?? ""),
        etag: effective.etag ?? "",
        status,
      });
    } catch {
      // 检查失败不打扰页面：徽标保持中性，打开面板时可重试。
      setSnap(null);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // 点浮层外部或 Esc 关闭。
  useEffect(() => {
    if (!open) return;
    const onPointer = (event: MouseEvent) => {
      if (anchorRef.current && !anchorRef.current.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const items = buildChecks(snap, () => void refresh(), (target) => {
    setOpen(false);
    navigate(target);
  });
  const pending = items.filter((item) => item.level === "bad" || item.level === "warn").length;
  const tone = !snap ? "muted" : pending > 0 ? "warn" : "ok";

  async function setDefaultModel(modelId: string) {
    if (!snap) return;
    setBusy(true);
    try {
      const changes = { "ai.defaultModelId": modelId };
      const checked = (await validateSettings(changes)) as { valid: boolean; fieldErrors: { field: string; message: string }[] };
      if (!checked.valid) {
        notify("bad", "设置校验未通过", checked.fieldErrors?.[0]?.message);
        return;
      }
      const operation = await applySettings(changes, "快速配置：设置默认模型", snap.etag);
      await run(operation, "应用系统设置", { silentSuccess: true });
      notify("ok", "默认模型已设置");
      await refresh();
    } catch (problem) {
      notify("bad", "设置默认模型失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="check-anchor" ref={anchorRef}>
      <button type="button" className={`check-pill is-${tone}`} onClick={() => { setOpen((v) => !v); if (!snap) void refresh(); }} aria-expanded={open}>
        <span className={`dot ${tone === "warn" ? "warn" : tone === "ok" ? "ok" : ""}`} />
        设置检查
        {snap && pending > 0 ? <span className="check-count">{pending}</span> : null}
        <IconChevronDown size={12} />
      </button>
      {open ? (
        <div className="check-pop" role="dialog" aria-label="设置检查">
          <div className="check-pop-head">
            <strong>运行必须项检查</strong>
            <button type="button" className="btn btn-ghost btn-sm" onClick={() => void refresh()} disabled={busy}>
              <IconRefresh size={13} /> 重新检查
            </button>
          </div>
          {!snap ? (
            <div className="check-empty">检查未完成，请重新检查。</div>
          ) : (
            <div className="check-list">
              {items.map((item) => (
                <div className={`check-item is-${item.level}`} key={item.id}>
                  <span className="check-mark">{item.level === "ok" ? <IconCheck size={13} /> : "!"}</span>
                  <span className="check-main">
                    <span className="check-label">{item.label}</span>
                    <span className="check-detail">{item.detail}</span>
                  </span>
                  {item.action ? (
                    <button type="button" className="btn btn-sm" disabled={busy} onClick={item.action.run}>
                      {item.action.label}
                    </button>
                  ) : null}
                </div>
              ))}
            </div>
          )}
          {snap && items.some((item) => item.id === "model" && item.level !== "ok") ? (
            <QuickModelForm
              busy={busy}
              onDone={async () => {
                await refresh();
              }}
            />
          ) : null}
          {snap && snap.models.length > 0 && !snap.defaultModelId ? (
            <DefaultModelForm busy={busy} models={snap.models} onPick={(id) => void setDefaultModel(id)} />
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function buildChecks(
  snap: Snapshot | null,
  retry: () => void,
  goto: (target: string) => void,
): CheckItem[] {
  if (!snap) {
    return [{ id: "load", label: "检查状态", level: "info", detail: "读取前置条件与系统状态失败。", action: { label: "重试", run: retry } }];
  }
  const scope = (id: string) => snap.preconditions.scopes.find((item) => item.scope === id);
  const llm = scope("llm");
  const collection = scope("collection");
  const search = scope("search");
  const items: CheckItem[] = [];

  // 模型：未登记是硬缺口；已登记但未设默认是建议项（加工链路按版本引用模型，默认值兜底）。
  if (!llm?.met) {
    items.push({ id: "model", label: "模型", level: "bad", detail: llm?.requirements[0]?.message ?? "尚未配置模型。" });
  } else if (!snap.defaultModelId) {
    items.push({ id: "model", label: "模型", level: "warn", detail: "已登记模型，未设置默认模型；未显式指定时缺少默认模型。" });
  } else {
    const named = snap.models.find((model) => model.id === snap.defaultModelId);
    items.push({ id: "model", label: "模型", level: "ok", detail: named ? `默认模型：${named.name}` : "默认模型已设置。" });
  }

  if (!collection?.met) {
    items.push({
      id: "collection", label: "信源", level: "bad",
      detail: collection?.requirements[0]?.message ?? "尚无启用的信源。",
      action: { label: "新建信源", run: () => goto("sources?new=1") },
    });
  } else {
    items.push({ id: "collection", label: "信源", level: "ok", detail: "至少一个信源处于启用状态。" });
  }

  if (!search?.met) {
    items.push({
      id: "search", label: "搜索引擎", level: "bad",
      detail: search?.requirements[0]?.message ?? "默认搜索引擎未就绪。",
      action: { label: "前往设置", run: () => goto("settings") },
    });
  } else {
    items.push({ id: "search", label: "搜索引擎", level: "ok", detail: `默认引擎：${search.engine ?? "ddg"}。` });
  }

  // 结构化解析依赖采集进程装载 rawprism 仓颉 Guest，属部署前提，无界面内快捷修复。
  if (snap.status.capabilities?.parse) {
    items.push({ id: "parse", label: "结构化解析", level: "ok", detail: "rawprism 已接入，解析型采集可用。" });
  } else {
    items.push({ id: "parse", label: "结构化解析", level: "info", detail: "rawprism 未接入：需在采集侧构建并装载仓颉 Guest 产物。" });
  }
  return items;
}

/// 预设快速登记模型：选平台带出端点与密钥环境变量名，填模型名后能力缺省按预设自动带出，
/// 密钥既可用环境变量也可以直接填写（直接填写只写不读）。
function QuickModelForm({ busy, onDone }: { busy: boolean; onDone: () => Promise<void> }) {
  const { notify } = useUi();
  const [presetId, setPresetId] = useState(MODEL_PRESETS[0].id);
  const preset = [...MODEL_PRESETS, CUSTOM_PRESET].find((item) => item.id === presetId) ?? MODEL_PRESETS[0];
  const [model, setModel] = useState(preset.models[0] ?? "");
  const [secretValue, setSecretValue] = useState("");
  const [pending, setPending] = useState(false);
  // 平台清单按当前端点与密钥实时拉取；预设只作为离线候选。
  const [remote, setRemote] = useState<string[]>([]);
  const [pulling, setPulling] = useState(false);
  // 能力缺省随模型名自动解析；未识别的型号走保守兜底值。
  const profile = resolveProfile(model);

  function pick(id: string) {
    setPresetId(id);
    const found = [...MODEL_PRESETS, CUSTOM_PRESET].find((item) => item.id === id);
    setModel(found?.models[0] ?? "");
    setSecretValue("");
    setRemote([]);
  }

  async function pull() {
    if (!preset.endpoint.trim()) {
      notify("bad", "无法拉取", "自定义端点需要先填写地址。");
      return;
    }
    setPulling(true);
    try {
      const page = await discoverModels({
        kind: preset.kind,
        endpoint: preset.endpoint.trim(),
        secretRef: preset.secretRef || null,
        secretValue: secretValue.trim() || null,
      });
      setRemote(page.items);
      if (!page.items.length) notify("warn", "平台没有返回模型", "确认密钥有列举权限。");
      else if (!model.trim()) setModel(page.items[0]);
    } catch (problem) {
      notify("bad", "拉取模型失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setPulling(false);
    }
  }

  async function submit() {
    if (!model.trim() || !preset.endpoint.trim()) {
      notify("bad", "无法登记", "端点与模型名不能为空。");
      return;
    }
    setPending(true);
    try {
      const inline = secretValue.trim();
      await createModel({
        name: `${preset.label} · ${model}`,
        kind: preset.kind,
        endpoint: preset.endpoint,
        secretRef: inline ? null : preset.secretRef || null,
        secretValue: inline || null,
        enabled: true,
        model,
        timeoutMs: 90_000,
        maxTokens: profile.maxTokens,
        temperature: profile.temperature,
        contextWindow: profile.contextWindow,
        reasoning: profile.reasoning,
        modalities: profile.modalities,
      });
      notify("ok", "模型已登记", `已按预设填充默认能力：${profileSummary(profile)}`);
      await onDone();
    } catch (problem) {
      notify("bad", "登记失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setPending(false);
    }
  }

  const busyNow = busy || pending;
  return (
    <div className="check-quick">
      <div className="check-quick-title">快速登记模型</div>
      <div className="check-quick-row">
        <select className="select" value={presetId} onChange={(event) => pick(event.target.value)} aria-label="平台预设" disabled={busyNow}>
          {[...MODEL_PRESETS, CUSTOM_PRESET].map((item) => (
            <option key={item.id} value={item.id}>{item.label}</option>
          ))}
        </select>
        <input
          className="input"
          list="check-model-candidates"
          value={model}
          placeholder="模型名"
          onChange={(event) => setModel(event.target.value)}
          disabled={busyNow}
        />
        <datalist id="check-model-candidates">
          {[...preset.models, ...remote].map((item) => <option key={item} value={item} />)}
        </datalist>
        <button type="button" className="btn btn-sm" disabled={busyNow || pulling} onClick={() => void pull()}>
          {pulling ? "拉取中…" : "拉取"}
        </button>
      </div>
      <div className="check-quick-row">
        <input
          className="input mono"
          type="password"
          autoComplete="off"
          value={secretValue}
          placeholder={preset.secretRef ? `密钥（留空则读环境变量 ${preset.secretRef}）` : "密钥（留空表示无需密钥）"}
          onChange={(event) => setSecretValue(event.target.value)}
          disabled={busyNow}
        />
      </div>
      <div className="check-quick-meta">
        <span className="mono small">{preset.endpoint || "自定义端点"}</span>
        <span className="small">{profileSummary(profile)}</span>
      </div>
      <button type="button" className="btn btn-primary btn-sm" disabled={busyNow} onClick={() => void submit()}>
        {busyNow ? "登记中…" : "登记模型"}
      </button>
    </div>
  );
}

function DefaultModelForm({ busy, models, onPick }: { busy: boolean; models: ModelResource[]; onPick: (id: string) => void }) {
  const candidates = models.filter((model) => model.enabled).concat(models.filter((model) => !model.enabled));
  const [picked, setPicked] = useState(candidates[0]?.id ?? "");
  return (
    <div className="check-quick">
      <div className="check-quick-title">设置默认模型</div>
      <div className="check-quick-row">
        <select className="select" value={picked} onChange={(event) => setPicked(event.target.value)} disabled={busy}>
          {candidates.map((model) => (
            <option key={model.id} value={model.id}>{model.name}{model.enabled ? "" : "（停用）"}</option>
          ))}
        </select>
        <button type="button" className="btn btn-primary btn-sm" disabled={busy || !picked} onClick={() => onPick(picked)}>
          {busy ? "应用中…" : "应用"}
        </button>
      </div>
    </div>
  );
}
