import { useCallback, useEffect, useMemo, useState } from "react";

import { Drawer } from "../components/Drawer";
import {
  IconAlert,
  IconCheck,
  IconConfig,
  IconFile,
  IconPlay,
  IconRefresh,
  IconSave,
  IconTrash,
} from "../components/icons";
import {
  Card,
  Collapse,
  Empty,
  FieldName,
  Pill,
  SectionTitle,
  Skeleton,
} from "../components/ui";
import {
  applyConfig,
  configDiff,
  configFile,
  configFiles,
  deleteConfigFile,
  explain,
  saveConfigFile,
  validateConfig,
} from "../lib/api";
import { useConsole } from "../lib/console";
import { bytes, kindLabel, relative, stamp } from "../lib/format";
import { matchConfigFile, setYamlLineValue } from "../lib/files";
import { navigate, useRoute } from "../lib/router";
import type { ConfigChange, ConfigFileBody, ConfigFileMeta, FieldOrigin, Module } from "../lib/types";

const KIND_ORDER = ["system", "source", "accessor", "adapter", "type", "preset"];

export function ConfigPage() {
  const { snapshot, state, notify, now, refresh } = useConsole();
  const { param } = useRoute();
  const [changes, setChanges] = useState<ConfigChange[] | null>(null);
  const [checked, setChecked] = useState<{ ok: boolean; at: number; message?: string } | null>(null);
  const [busy, setBusy] = useState<"validate" | "diff" | null>(null);
  const [selected, setSelected] = useState<Module | null>(null);
  // 声明文件原文属于高级配置：默认收起，从信源抽屉带着文件路径跳来时才自动展开。
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const loading = state === "loading";

  useEffect(() => {
    if (param) {
      setAdvancedOpen(true);
    }
  }, [param]);

  const modules = snapshot?.catalog.modules ?? [];
  const system = modules.find((module) => module.kind === "system") ?? null;
  const presetId = text(system?.config, ["policy", "preset"]);
  const preset = modules.find((module) => module.id === `preset/${presetId}`) ?? null;
  const generation = snapshot?.catalog.generation ?? null;

  const grouped = useMemo(() => {
    return KIND_ORDER.map((kind) => ({
      kind,
      items: modules.filter((module) => module.kind === kind),
    })).filter((group) => group.items.length > 0);
  }, [modules]);

  async function runValidate() {
    setBusy("validate");
    try {
      await validateConfig();
      setChecked({ ok: true, at: Date.now() });
      notify("ok", "配置校验通过", "目录里的 YAML 能合并成一份可运行的快照。");
      const next = await configDiff();
      setChanges(next);
    } catch (problem) {
      const message = problem instanceof Error ? problem.message : "校验失败";
      setChecked({ ok: false, at: Date.now(), message });
      notify("bad", "配置没有通过校验", message);
    } finally {
      setBusy(null);
    }
  }

  async function runApply() {
    setBusy("validate");
    try {
      const next = await applyConfig();
      setChecked({ ok: true, at: Date.now() });
      notify("ok", `配置已生效，代际 G-${next}`, "采集进程已重新加载目录里的声明文件。");
      const [diff] = await Promise.all([configDiff(), refresh()]);
      setChanges(diff);
    } catch (problem) {
      const message = problem instanceof Error ? problem.message : "应用失败";
      setChecked({ ok: false, at: Date.now(), message });
      notify("bad", "配置没有生效", message);
    } finally {
      setBusy(null);
    }
  }

  async function runDiff() {
    setBusy("diff");
    try {
      const next = await configDiff();
      setChanges(next);
      notify(
        next.length === 0 ? "ok" : "info",
        next.length === 0 ? "草稿与运行中的配置一致" : `有 ${next.length} 处待生效的改动`,
        next.length === 0 ? "没有需要重新加载的内容。" : "采集进程每 2 秒扫描一次目录，保存后会自动生效。",
      );
    } catch (problem) {
      notify("bad", "无法比较改动", problem instanceof Error ? problem.message : undefined);
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="page">
      <header className="page-head">
        <div className="page-head-text">
          <span className="page-kicker">config · 系统配置</span>
          <h1 className="page-title">声明文件、校验与生效</h1>
          <p className="page-lede">
            配置是一组 YAML 文件：可以在这里直接浏览与编辑，保存后约 2 秒热加载；也可以点「应用配置」立即生效。下方展示合并之后的生效值，以及每个值出自哪一行。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={runDiff} disabled={busy !== null}>
            <IconRefresh size={13} />
            {busy === "diff" ? "比较中…" : "比较草稿"}
          </button>
          <button type="button" className="btn btn-primary" onClick={runValidate} disabled={busy !== null}>
            <IconCheck size={13} />
            {busy === "validate" ? "校验中…" : "校验配置"}
          </button>
          <button
            type="button"
            className="btn btn-primary"
            onClick={runApply}
            disabled={busy !== null || (changes !== null && changes.length === 0)}
            title={changes && changes.length === 0 ? "目录与运行中的配置一致，无需应用" : "让采集进程立即重新加载配置"}
          >
            <IconPlay size={13} />
            应用配置
          </button>
        </div>
      </header>

      <div className="stat-grid">
        <div className="stat">
          <span className="stat-label">配置代际</span>
          <span className="stat-value">G-{generation ?? "—"}</span>
          <span className="stat-foot">每次成功加载 +1</span>
        </div>
        <div className="stat">
          <span className="stat-label">生效模块</span>
          <span className="stat-value">{loading ? "—" : modules.length}</span>
          <span className="stat-foot">
            信源 {modules.filter((module) => module.kind === "source").length} · 通道{" "}
            {modules.filter((module) => module.kind === "accessor").length} · 解析器{" "}
            {modules.filter((module) => module.kind === "adapter").length}
          </span>
        </div>
        <div className="stat">
          <span className="stat-label">当前预设</span>
          <span className="stat-value" style={{ fontFamily: "var(--sans)", fontSize: 18 }}>
            {presetId ?? "未指定"}
          </span>
          <span className="stat-foot">{preset ? preset.name ?? preset.id : "策略由 system.yaml 决定"}</span>
        </div>
        <div className={`stat${changes && changes.length > 0 ? " is-alert" : ""}`}>
          <span className="stat-label">待生效改动</span>
          <span className="stat-value">{changes === null ? "—" : changes.length}</span>
          <span className="stat-foot">
            {checked ? (checked.ok ? `上次校验 ${relative(new Date(checked.at).toISOString(), now)}通过` : "上次校验未通过") : "尚未校验"}
          </span>
        </div>
      </div>

      {checked ? (
        <div className={`note${checked.ok ? "" : " is-bad"}`}>
          {checked.ok ? <IconCheck size={14} /> : <IconAlert size={14} />}
          <div style={{ display: "grid", gap: 2 }}>
            <strong>{checked.ok ? "目录里的配置可以运行" : "目录里的配置无法运行"}</strong>
            <span>{checked.ok ? `校验于 ${stamp(new Date(checked.at).toISOString())}，快照仍然是 G-${generation ?? "—"}。` : checked.message}</span>
          </div>
        </div>
      ) : null}

      <Card
        title="待生效改动"
        sub={changes === null ? "点「比较草稿」看目录与运行中配置的差异" : `${changes.length} 处`}
        tight
      >
        {changes === null ? (
          <Empty title="还没有比较过" text="比较会重新读取目录，但不会替换正在运行的快照。" />
        ) : changes.length === 0 ? (
          <Empty title="完全一致" text="磁盘上的配置与正在运行的快照没有差异。" />
        ) : (
          <div className="diff-list">
            {changes.slice(0, 40).map((change) => (
              <div className="diff-row" key={change.path}>
                <span className="diff-path">{change.path}</span>
                <span className="diff-values">
                  <span className="diff-before">{short(change.before)}</span>
                  <span aria-hidden="true">→</span>
                  <span className="diff-after">{short(change.after)}</span>
                </span>
              </div>
            ))}
          </div>
        )}
      </Card>

      <Collapse
        title="高级配置 · 声明文件原文"
        note="直接编辑 YAML；日常调整建议用上方的模块卡片"
        open={advancedOpen}
        onToggle={setAdvancedOpen}
      >
        <FileWorkbench
          openFile={param}
          onOpened={() => navigate("config")}
          onSaved={async () => {
            try {
              await validateConfig();
              setChecked({ ok: true, at: Date.now() });
            } catch (problem) {
              const message = problem instanceof Error ? problem.message : "校验失败";
              setChecked({ ok: false, at: Date.now(), message });
            }
            try {
              setChanges(await configDiff());
            } catch {
              /* 差异稍后再看 */
            }
          }}
        />
      </Collapse>

      {preset ? <PolicyCard preset={preset} onOpen={() => setSelected(preset)} /> : null}

      {grouped
        .filter((group) => group.kind !== "system" && group.kind !== "preset")
        .map((group) => (
          <div key={group.kind}>
            <SectionTitle title={kindLabel(group.kind)} note={`${group.items.length} 个`} />
            <div className="config-grid">
              {group.items.map((module) => (
                <button
                  key={module.id}
                  type="button"
                  className="module-card"
                  onClick={() => setSelected(module)}
                >
                  <span className="module-top">
                    <span className="module-name">{module.name ?? module.id}</span>
                    <Pill tone={module.enabled ? "ok" : "idle"} dot>
                      {module.enabled ? "生效" : "停用"}
                    </Pill>
                  </span>
                  <span className="module-id">{module.id}</span>
                  <span className="module-origin">
                    {module.file.split(/[\\/]/).slice(-2).join("/")}:{module.line}
                  </span>
                </button>
              ))}
            </div>
          </div>
        ))}

      {loading ? <Skeleton lines={6} /> : null}

      <p className="faint" style={{ fontSize: 11.5, lineHeight: 1.7 }}>
        <IconConfig size={11} /> 配置以目录为唯一来源：保存后自动热加载，「应用配置」立即推进代际。这里同时展示生效值与来源。
      </p>

      <ModuleDrawer module={selected} onClose={() => setSelected(null)} onSaved={refresh} />
    </div>
  );
}

const POLICIES: { group: string; rows: { path: string[]; label: string; hint?: string }[] }[] = [
  {
    group: "保存与版本",
    rows: [
      { path: ["raw", "store"], label: "原始响应", hint: "是否留下原始字节" },
      { path: ["raw", "retain"], label: "原始保留期" },
      { path: ["media", "mode"], label: "媒体文件" },
      { path: ["media", "max_size"], label: "单个媒体上限" },
      { path: ["versions", "snapshot_retain"], label: "快照保留期" },
      { path: ["commit", "flush_interval"], label: "提交间隔" },
      { path: ["state", "snapshot_interval"], label: "状态快照间隔" },
    ],
  },
  {
    group: "访问与站点守则",
    rows: [
      { path: ["robots", "mode"], label: "robots 策略", hint: "是否遵守站点声明" },
      { path: ["robots", "cache_ttl"], label: "守则缓存" },
      { path: ["robots", "on_unreachable"], label: "守则读不到时" },
      { path: ["robots", "crawl_delay"], label: "抓取间隔" },
      { path: ["render", "max_level"], label: "渲染层级", hint: "需要时启用浏览器" },
      { path: ["capture", "screenshot"], label: "失败留证" },
    ],
  },
  {
    group: "归并与去重",
    rows: [
      { path: ["identity", "merge_primary"], label: "同一条内容的判定" },
      { path: ["identity", "fragment"], label: "页面锚点" },
      { path: ["identity", "strip_params"], label: "忽略参数" },
      { path: ["dedupe", "near_duplicate"], label: "近似去重" },
      { path: ["reprocess", "on_adapter_change"], label: "解析规则变更后" },
      { path: ["backfill", "initial", "limit"], label: "首次回填条数" },
    ],
  },
  {
    group: "费用与健康",
    rows: [
      { path: ["paid", "unknown_receipt"], label: "付费回执不确定时" },
      { path: ["health", "expected_period"], label: "预期产出周期" },
      { path: ["health", "failure_threshold"], label: "连续失败阈值" },
      { path: ["health", "yield_drop_ratio"], label: "产出下降告警线" },
    ],
  },
];

function PolicyCard({ preset, onOpen }: { preset: Module; onOpen: () => void }) {
  return (
    <Card
      title={`系统策略 · ${preset.name ?? preset.id}`}
      sub="由预设与 system.yaml 合并而成，改预设会同时影响所有信源"
      action={
        <button type="button" className="btn btn-quiet btn-sm" onClick={onOpen}>
          查看原文
        </button>
      }
      tight
    >
      {POLICIES.map((group) => (
        <div key={group.group} style={{ borderBottom: "1px solid var(--line-faint)" }}>
          <div style={{ padding: "10px 16px 2px" }}>
            <span className="section-title">{group.group}</span>
          </div>
          <div className="policy-grid">
            {group.rows.map((row) => {
              const value = walk(preset.config, ["policy", ...row.path]);
              return (
                <div className="policy-item" key={row.path.join(".")}>
                  <span className="policy-label">{row.label}</span>
                  <span className="policy-value">{describe(row.path, value)}</span>
                  {row.hint ? <span className="policy-hint">{row.hint}</span> : null}
                </div>
              );
            })}
          </div>
        </div>
      ))}
    </Card>
  );
}

function ModuleDrawer({
  module,
  onClose,
  onSaved,
}: {
  module: Module | null;
  onClose: () => void;
  onSaved: () => void;
}) {
  // 字段来源按模块 id 缓存：重开同一个模块、或写回后刷新时都不回退到骨架，避免闪烁。
  const [originState, setOriginState] = useState<{ id: string; fields: FieldOrigin[] } | null>(null);
  const [mergedOpen, setMergedOpen] = useState(false);
  const moduleId = module?.id ?? null;
  const origins =
    originState && moduleId !== null && originState.id === moduleId ? originState.fields : null;

  useEffect(() => {
    if (!moduleId) {
      return;
    }
    let alive = true;
    explain(moduleId)
      .then((fields) => {
        if (alive) {
          setOriginState({ id: moduleId, fields });
        }
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [moduleId]);

  if (!module) {
    return null;
  }

  /** 字段写回后重新拉取解释，让新值与行号立即可见；不清空旧数据，避免闪烁。 */
  const refreshOrigins = () => {
    if (!module) {
      return;
    }
    explain(module.id)
      .then((fields) => setOriginState({ id: module.id, fields }))
      .catch(() => undefined);
  };

  return (
    <Drawer
      key={module.id}
      open
      kicker={`${kindLabel(module.kind)} · 配置`}
      title={module.name ?? module.id}
      onClose={onClose}
    >
      <div className="row">
        <Pill tone={module.enabled ? "ok" : "idle"} dot>
          {module.enabled ? "生效" : "停用"}
        </Pill>
        <Pill>{module.id}</Pill>
        <Pill tone="info">
          {module.file}:{module.line}
        </Pill>
      </div>

      {module.description ? (
        <p className="muted" style={{ fontSize: 12.5 }}>
          {module.description}
        </p>
      ) : null}

      <div>
        <SectionTitle title="字段来源" note="点值可修改，来源是写入位置" />
        {origins === null ? (
          <Skeleton lines={5} />
        ) : origins.length === 0 ? (
          <Empty title="没有可解释的字段" />
        ) : (
          <div className="origin-list is-editable">
            {origins.slice(0, 60).map((field) => (
              <OriginField
                key={field.path}
                field={field}
                onChanged={() => {
                  refreshOrigins();
                  onSaved();
                }}
              />
            ))}
          </div>
        )}
      </div>

      <Collapse
        title="合并后的配置"
        note="预设与 system.yaml 叠加后的最终值"
        open={mergedOpen}
        onToggle={setMergedOpen}
      >
        <pre className="code">{JSON.stringify(module.config, null, 2)}</pre>
      </Collapse>
    </Drawer>
  );
}

/** 字段来源行：标量值可以行内编辑，写回定义处那一行；对象与数组只读。 */
function OriginField({ field, onChanged }: { field: FieldOrigin; onChanged: () => void }) {
  const { notify } = useConsole();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [saving, setSaving] = useState(false);
  const scalar =
    typeof field.value === "string" || typeof field.value === "number" || typeof field.value === "boolean";
  const key = field.path.split(".").pop() ?? field.path;

  async function write(value: string | number | boolean) {
    setSaving(true);
    try {
      const files = await configFiles();
      const path = matchConfigFile(files, field.file);
      if (!path) {
        throw new Error(`${field.file} 不在配置目录里`);
      }
      const body = await configFile(path);
      const next = setYamlLineValue(body.content, field.line, key, value);
      if (next === body.content) {
        notify("info", "值没有变化");
        setEditing(false);
        return;
      }
      await saveConfigFile(path, next);
      notify("ok", `已更新 ${field.path}`, `${shortFile(field.file)}:${field.line} 已写回，约 2 秒内热加载。`);
      setEditing(false);
      onChanged();
    } catch (problem) {
      notify("bad", "写入失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setSaving(false);
    }
  }

  async function commit() {
    if (typeof field.value === "number") {
      const parsed = Number(draft.trim());
      if (draft.trim() === "" || Number.isNaN(parsed)) {
        notify("bad", "需要数字", `${key} 应该是数字，当前是 ${field.value}`);
        return;
      }
      await write(parsed);
      return;
    }
    if (typeof field.value === "boolean") {
      await write(draft === "true");
      return;
    }
    await write(draft);
  }

  return (
    <div className="origin-row">
      <FieldName path={field.path} />
      <span className="origin-value">
        {typeof field.value === "boolean" ? (
          <span className="origin-switch">
            <button
              type="button"
              className={`origin-opt${field.value ? " is-on" : ""}`}
              disabled={saving || field.value}
              onClick={() => void write(true)}
            >
              开
            </button>
            <button
              type="button"
              className={`origin-opt${!field.value ? " is-on" : ""}`}
              disabled={saving || !field.value}
              onClick={() => void write(false)}
            >
              关
            </button>
          </span>
        ) : editing ? (
          <span className="origin-edit">
            <input
              value={draft}
              autoFocus
              spellCheck={false}
              disabled={saving}
              onChange={(event) => setDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  void commit();
                }
                if (event.key === "Escape") {
                  setEditing(false);
                }
              }}
            />
            <button type="button" className="origin-opt is-on" disabled={saving} onClick={() => void commit()}>
              <IconCheck size={11} />
            </button>
            <button type="button" className="origin-opt" disabled={saving} onClick={() => setEditing(false)}>
              ×
            </button>
          </span>
        ) : scalar ? (
          <button
            type="button"
            className="origin-edit-btn"
            onClick={() => {
              setDraft(String(field.value));
              setEditing(true);
            }}
            title="点击修改"
          >
            {describe(field.path.split("."), field.value)}
          </button>
        ) : (
          <span className="origin-plain">{describe(field.path.split("."), field.value)}</span>
        )}
      </span>
      <span
        className={`origin-source${field.viaPreset ? " is-preset" : ""}`}
        title={`${field.file}:${field.line}`}
      >
        {field.viaPreset ? `预设 ${field.presetId ?? ""}` : shortFile(field.file)}:{field.line}
      </span>
    </div>
  );
}

function walk(config: unknown, path: string[]): unknown {
  let current: unknown = config;
  for (const key of path) {
    if (!current || typeof current !== "object") {
      return undefined;
    }
    current = (current as Record<string, unknown>)[key];
  }
  return current;
}

function text(config: unknown, path: string[]): string | null {
  const value = walk(config, path);
  return typeof value === "string" ? value : null;
}

const VALUES: Record<string, string> = {
  all: "全部保存",
  metadata: "只留元数据",
  download: "下载",
  link: "只留链接",
  obey: "遵守",
  "obey-with-exemptions": "遵守（白名单例外）",
  ignore: "忽略",
  rfc9309: "按 RFC 9309 视为禁止",
  allow: "放行",
  disallow: "拦截",
  none: "不启用",
  "retry-once": "重试一次",
  manual: "人工确认",
  simhash: "近似去重",
  basic: "基础分类",
  "on-failure": "失败时留截图",
  off: "关闭",
  L1: "仅静态请求",
  L2: "可选浏览器",
  L3: "允许完整渲染",
  latest: "只重算最新版本",
  "external-id-first": "优先用站点 ID 归并",
  drop: "丢弃",
  "cap:30s": "最多等 30 秒",
};

function describe(path: string[], value: unknown): string {
  if (value === undefined || value === null) {
    return "未设置";
  }
  if (typeof value === "object") {
    if (Array.isArray(value)) {
      return value.length ? value.map((item) => (typeof item === "string" ? VALUES[item] ?? item : String(item))).join("、") : "无";
    }
    return Object.entries(value as Record<string, unknown>)
      .map(([key, item]) => `${key}: ${String(item)}`)
      .join("  ");
  }
  if (typeof value === "boolean") {
    return value ? "开启" : "关闭";
  }
  if (typeof value === "number") {
    const key = path[path.length - 1];
    return key === "yield_drop_ratio" || key === "yield-drop-ratio"
      ? `${Math.round(value * 100)}%`
      : String(value);
  }
  return VALUES[String(value)] ?? String(value);
}

function short(value: unknown): string {
  if (value === null || value === undefined) {
    return "（删除）";
  }
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return text.length > 46 ? `${text.slice(0, 46)}…` : text;
}

function shortFile(file: string): string {
  const parts = file.split(/[\\/]/);
  return parts.length <= 2 ? file : parts.slice(-2).join("/");
}

/** 配置文件工作台：浏览目录、编辑、保存（写回即热加载）、删除与新建。 */
function FileWorkbench({
  onSaved,
  openFile,
  onOpened,
}: {
  onSaved: () => Promise<void>;
  openFile?: string | null;
  onOpened?: () => void;
}) {
  const { notify } = useConsole();
  const [files, setFiles] = useState<ConfigFileMeta[] | null>(null);
  const [body, setBody] = useState<ConfigFileBody | null>(null);
  const [draft, setDraft] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const dirty = body !== null && draft !== body.content;

  const load = useCallback(async (pick?: string) => {
    try {
      const next = await configFiles();
      setFiles(next);
      setError(null);
      if (pick) {
        const opened = await configFile(pick);
        setBody(opened);
        setDraft(opened.content);
      }
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : "读取失败");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // 从别的页面（如信源抽屉）带着文件路径跳过来时，直接打开它。
  useEffect(() => {
    if (!openFile || files === null) {
      return;
    }
    const hit = matchConfigFile(files, openFile);
    if (hit) {
      void select(hit);
    } else {
      notify("bad", "没有找到配置文件", `${openFile} 不在配置目录里。`);
    }
    onOpened?.();
  }, [openFile, files, notify, onOpened]);

  async function select(path: string) {
    if (path === body?.path) {
      return;
    }
    if (dirty && !window.confirm("当前文件有未保存的修改，放弃并打开新文件？")) {
      return;
    }
    try {
      const opened = await configFile(path);
      setBody(opened);
      setDraft(opened.content);
      setError(null);
    } catch (problem) {
      notify("bad", "无法读取文件", problem instanceof Error ? problem.message : undefined);
    }
  }

  async function save() {
    if (!body) {
      return;
    }
    setSaving(true);
    try {
      await saveConfigFile(body.path, draft);
      notify("ok", `已保存 ${body.path}`, "采集进程约 2 秒内热加载；点「校验配置」确认可用。");
      await load(body.path);
      await onSaved();
    } catch (problem) {
      notify("bad", "保存失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setSaving(false);
    }
  }

  async function create() {
    const input = window.prompt("新文件相对路径（以 yaml/yml/json 结尾，如 sources/example.yaml）");
    if (!input) {
      return;
    }
    const path = input.trim().replace(/\\/g, "/");
    if (files?.some((file) => file.path === path)) {
      notify("bad", "文件已存在", "在左侧列表里选中它直接编辑即可。");
      return;
    }
    try {
      await saveConfigFile(path, "");
      notify("ok", `已创建 ${path}`, "空文件也参与合并与校验，建议先填好再应用。");
      await load(path);
      await onSaved();
    } catch (problem) {
      notify("bad", "创建失败", problem instanceof Error ? problem.message : undefined);
    }
  }

  async function remove() {
    if (!body) {
      return;
    }
    if (!window.confirm(`删除 ${body.path}？该操作不可撤销。`)) {
      return;
    }
    try {
      await deleteConfigFile(body.path);
      notify("ok", `已删除 ${body.path}`, "下一轮热加载后，对应模块会从目录里消失。");
      setBody(null);
      setDraft("");
      await load();
      await onSaved();
    } catch (problem) {
      notify("bad", "删除失败", problem instanceof Error ? problem.message : undefined);
    }
  }

  return (
    <Card
      title="配置文件"
      sub="浏览 · 编辑 · 保存即热加载"
      action={
        <button type="button" className="btn btn-sm" onClick={() => void create()}>
          <IconFile size={13} /> 新建文件
        </button>
      }
      tight
    >
      {files === null ? (
        <Skeleton lines={6} />
      ) : files.length === 0 ? (
        <Empty title="配置目录是空的" text="检查 modalprism-console 启动参数 --config 指向的目录。" />
      ) : (
        <div className="workbench">
          <ul className="workbench-list">
            {files.map((file) => (
              <li key={file.path}>
                <button
                  type="button"
                  className={`workbench-item${body?.path === file.path ? " is-active" : ""}`}
                  onClick={() => void select(file.path)}
                >
                  <span className="mono workbench-path">{file.path}</span>
                  <span className="faint workbench-meta">
                    {bytes(file.size)}
                    {file.modifiedMs ? ` · ${relative(new Date(file.modifiedMs).toISOString())}` : ""}
                  </span>
                </button>
              </li>
            ))}
          </ul>
          <div className="workbench-editor">
            {error ? <div className="note is-bad"><strong>读取失败</strong><span>{error}</span></div> : null}
            {body === null ? (
              <Empty title="选择一个文件" text="左侧是配置目录里的 YAML / JSON 文件，选中后可直接编辑。" />
            ) : (
              <>
                <div className="workbench-bar">
                  <span className="mono cell-strong">{body.path}</span>
                  {dirty ? <Pill tone="warn">未保存</Pill> : <Pill tone="ok">已同步</Pill>}
                  <span className="workbench-actions">
                    {dirty ? (
                      <button type="button" className="btn btn-sm btn-quiet" onClick={() => setDraft(body.content)} disabled={saving}>
                        还原
                      </button>
                    ) : null}
                    <button type="button" className="btn btn-sm btn-quiet" onClick={() => void remove()} disabled={saving}>
                      <IconTrash size={13} /> 删除
                    </button>
                    <button type="button" className="btn btn-sm btn-primary" onClick={() => void save()} disabled={saving || !dirty}>
                      <IconSave size={13} /> {saving ? "保存中…" : "保存"}
                    </button>
                  </span>
                </div>
                <textarea
                  className="workbench-textarea mono"
                  value={draft}
                  spellCheck={false}
                  onChange={(event) => setDraft(event.target.value)}
                />
                <p className="faint workbench-hint">
                  保存会写回目录里的文件，采集进程约 2 秒内热加载；代际是否已推进，看页面顶部「配置代际」。
                </p>
              </>
            )}
          </div>
        </div>
      )}
    </Card>
  );
}
