// 内容加工的编辑抽屉：工作流、模型与运行记录。
//
// 三张表单共用同一套原语（Field / TextField / Switch …），页面只负责列表与动作。
// 声明形状与采集进程里的 `WorkflowSpec` / `ModelSpec` 一一对应；字段校验留在服务端，
// 这里只组装草稿，并给每个步骤留一个整份 JSON 的出口。

import { useState } from "react";

import { Drawer } from "./Drawer";
import { Choice, Field, NumberField, SelectField, Switch, TextArea, TextField } from "./forms";
import { IconAlert, IconPlus, IconTrash } from "./icons";
import { useConsole } from "../lib/console";
import {
  MODEL_MODES,
  NOTIFY_KINDS,
  STEP_KINDS,
  TRIGGER_ONS,
  blankModel,
  blankStep,
  blankWorkflow,
  joinList,
  modeLabel,
  remove,
  removeModel,
  run as runNow,
  save,
  saveModel,
  splitList,
  statusLabel,
  stepKindLabel,
  type ModelDoc,
  type ModelRow,
  type ModelTemplate,
  type NotifyKind,
  type StepKind,
  type WorkflowDoc,
  type WorkflowNotify,
  type WorkflowRow,
  type WorkflowRun,
  type WorkflowStep,
  type WorkflowTemplate,
} from "../lib/workflows";

const FACET_KINDS = [
  { value: "md", label: "Markdown" },
  { value: "text", label: "纯文本" },
  { value: "struct", label: "结构化（struct）" },
];

// ── 工作流 ──────────────────────────────────────────────────────────────────

export function WorkflowDrawer({
  item,
  models,
  templates,
  onClose,
  onSaved,
}: {
  item: WorkflowRow | null;
  models: ModelRow[];
  templates: WorkflowTemplate[];
  onClose: () => void;
  onSaved: (message: string) => void;
}) {
  const { notify } = useConsole();
  const editing = item !== null;
  const usable = models.filter((model) => !model.error);
  const [slug, setSlug] = useState(item?.slug ?? "");
  const [doc, setDoc] = useState<WorkflowDoc>(
    () => item?.document ?? blankWorkflow(usable[0]?.slug ?? "default"),
  );
  const [advanced, setAdvanced] = useState(false);
  const [draft, setDraft] = useState(() => JSON.stringify(item?.document ?? doc, null, 2));
  const [saving, setSaving] = useState(false);
  const [running, setRunning] = useState(false);

  function patch(next: Partial<WorkflowDoc>) {
    setDoc((current) => ({ ...current, ...next }));
  }

  function patchTrigger(next: Partial<NonNullable<WorkflowDoc["trigger"]>>) {
    setDoc((current) => ({ ...current, trigger: { ...current.trigger, ...next } }));
  }

  function patchStep(index: number, next: Partial<WorkflowStep>) {
    setDoc((current) => ({
      ...current,
      steps: current.steps.map((step, at) => (at === index ? { ...step, ...next } : step)),
    }));
  }

  function changeStepKind(index: number, kind: StepKind) {
    const current = doc.steps[index];
    setDoc((state) => ({
      ...state,
      steps: state.steps.map((step, at) => (at === index ? { id: current.id, kind, ...seedOf(kind) } : step)),
    }));
  }

  function addStep() {
    setDoc((current) => ({ ...current, steps: [...current.steps, blankStep(current.steps.length + 1)] }));
  }

  function moveStep(index: number, direction: number) {
    const target = index + direction;
    if (target < 0 || target >= doc.steps.length) {
      return;
    }
    setDoc((current) => {
      const steps = [...current.steps];
      [steps[index], steps[target]] = [steps[target], steps[index]];
      return { ...current, steps };
    });
  }

  function dropStep(index: number) {
    setDoc((current) => ({ ...current, steps: current.steps.filter((_, at) => at !== index) }));
  }

  function applyTemplate(template: WorkflowTemplate) {
    setDoc((current) => ({
      ...template.doc,
      model: current.model || template.doc.model,
      enabled: true,
    }));
    setDraft(JSON.stringify(template.doc, null, 2));
    notify("info", `已套用模板「${template.name}」`, "保存前可以继续改步骤与提示词。");
  }

  function patchNotify(index: number, next: Partial<WorkflowNotify>) {
    setDoc((current) => ({
      ...current,
      notify: (current.notify ?? []).map((row, at) => (at === index ? { ...row, ...next } : row)),
    }));
  }

  function addNotify() {
    setDoc((current) => ({
      ...current,
      notify: [...(current.notify ?? []), { kind: "feishu", url: "", on: ["failure"] }],
    }));
  }

  function dropNotify(index: number) {
    setDoc((current) => ({ ...current, notify: (current.notify ?? []).filter((_, at) => at !== index) }));
  }

  function applyDraft() {
    try {
      const parsed = JSON.parse(draft) as WorkflowDoc;
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error("声明必须是对象");
      }
      setDoc(parsed);
      notify("ok", "已套用到表单", "保存时会再走一次完整校验。");
    } catch (problem) {
      notify("bad", "JSON 无法解析", problem instanceof Error ? problem.message : undefined);
    }
  }

  async function submit() {
    if (!slug.trim()) {
      notify("bad", "还缺工作流名", "它同时是文件名，只能用字母、数字、点、下划线与连字符。");
      return;
    }
    if (!usable.length && !editing) {
      notify("bad", "还没有可用的模型", "先到「模型」页建一个，再回来建工作流。");
      return;
    }
    setSaving(true);
    try {
      const saved = await save(slug.trim(), doc);
      onSaved(editing ? `已更新 ${saved.file}` : `已新建 ${saved.file}`);
      onClose();
    } catch (problem) {
      notify("bad", "没有通过校验", problem instanceof Error ? problem.message : undefined);
    } finally {
      setSaving(false);
    }
  }

  async function discard() {
    if (!item || !window.confirm(`删除 ${item.file}？运行记录会保留在内容分层里。`)) {
      return;
    }
    setSaving(true);
    try {
      await remove(item.slug);
      onSaved(`已删除 ${item.file}`);
      onClose();
    } catch (problem) {
      notify("bad", "删除失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setSaving(false);
    }
  }

  async function runOnce() {
    const rid = window.prompt("要处理哪一条内容？填入 rid（内容库抽屉里可以复制）");
    if (!rid?.trim()) {
      return;
    }
    setRunning(true);
    try {
      const result = await runNow(item!.slug, rid.trim());
      const failed = result.steps.filter((step) => step.status !== "ok").length;
      notify(
        result.status === "ok" ? "ok" : "warn",
        result.status === "ok" ? "这一条处理完成" : "处理失败",
        result.error ?? `${result.steps.length} 个步骤，${failed} 个未通过，用时 ${result.durationMs} ms`,
      );
    } catch (problem) {
      notify("bad", "运行失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setRunning(false);
    }
  }

  const facet = doc.outputs?.facet;
  const notifies = doc.notify ?? [];

  return (
    <Drawer
      open
      kicker={editing ? `工作流 · ${item?.file}` : "工作流 · 新建"}
      title={editing ? (item?.name ?? item?.slug ?? "") : "新建工作流"}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn btn-primary" disabled={saving} onClick={() => void submit()}>
            {saving ? "写入中…" : editing ? "保存并校验" : "创建并校验"}
          </button>
          {editing ? (
            <button type="button" className="btn" disabled={running || saving} onClick={() => void runOnce()}>
              {running ? "运行中…" : "手动运行"}
            </button>
          ) : null}
          {editing ? (
            <button type="button" className="btn btn-quiet" disabled={saving} onClick={() => void discard()}>
              <IconTrash size={13} /> 删除
            </button>
          ) : null}
          <span className="spacer" />
          <button type="button" className="btn btn-quiet btn-sm" onClick={onClose}>
            取消
          </button>
        </>
      }
    >
      {!editing && templates.length ? (
        <div className="stack-sm">
          <span className="form-label">从模板起步</span>
          <div className="template-grid">
            {templates.map((template) => (
              <button
                key={template.slug}
                type="button"
                className="template-card"
                onClick={() => applyTemplate(template)}
              >
                <span className="template-name">{template.name}</span>
                <span className="template-desc">{template.description}</span>
                <span className="template-tag mono">workflow/{template.slug}</span>
              </button>
            ))}
          </div>
        </div>
      ) : null}

      <div className="form-grid">
        <Field label="工作流名" hint={editing ? "新建后固定，要改名请另建一份" : "成为文件名与 id，例如 digest"}>
          <TextField value={slug} onChange={setSlug} placeholder="digest" mono disabled={editing} />
        </Field>
        <Field label="状态" hint="停用后不再自动处理新内容，手动运行仍可用">
          <Switch checked={doc.enabled !== false} onChange={(next) => patch({ enabled: next })} label="启用工作流" />
        </Field>
        <Field label="名称" hint="界面与通知里显示的名字">
          <TextField value={doc.name ?? ""} onChange={(next) => patch({ name: next })} placeholder="要点摘要" />
        </Field>
        <Field label="默认模型" hint={usable.length ? "步骤可以各自覆盖" : "还没有可用模型，先去「模型」页创建"}>
          <SelectField
            value={doc.model}
            onChange={(next) => patch({ model: next })}
            options={usable.map((model) => ({
              value: model.slug,
              label: `${model.name} · ${modeLabel(model.mode)}`,
            }))}
            placeholder={usable.length ? "选择模型" : "没有可用模型"}
          />
        </Field>
        <Field label="说明" hint="一句话描述它做什么" wide>
          <TextField value={doc.description ?? ""} onChange={(next) => patch({ description: next })} />
        </Field>
        <Field label="触发方式" hint="新内容入库时触发，或只留手动运行">
          <SelectField
            value={doc.trigger?.on ?? "commit"}
            onChange={(next) => patchTrigger({ on: next as "commit" | "manual" })}
            options={TRIGGER_ONS}
          />
        </Field>
        <Field label="资源类型" hint="逗号分隔，默认 article">
          <TextField
            value={joinList(doc.trigger?.kinds)}
            onChange={(next) => patchTrigger({ kinds: splitList(next) })}
            placeholder="article"
            mono
          />
        </Field>
        <Field label="限定信源" hint="留空表示全部信源；写信源 id，逗号分隔" wide>
          <TextField
            value={joinList(doc.trigger?.sources)}
            onChange={(next) => patchTrigger({ sources: splitList(next) })}
            placeholder="arxiv-ai, hugging-face"
            mono
          />
        </Field>
      </div>

      <div className="stack-sm">
        <div className="section-head">
          <span className="section-title">步骤</span>
          <button type="button" className="btn btn-sm" onClick={addStep}>
            <IconPlus size={12} /> 添加步骤
          </button>
        </div>
        {doc.steps.length === 0 ? <p className="faint">至少需要一个步骤，否则保存会被拦下。</p> : null}
        {doc.steps.map((step, index) => (
          <div className="wf-step" key={`${step.id}-${index}`}>
            <div className="wf-step-head">
              <span className="wf-step-index mono">{String(index + 1).padStart(2, "0")}</span>
              <TextField
                value={step.id}
                onChange={(next) => patchStep(index, { id: next })}
                placeholder="步骤 id"
                mono
              />
              <SelectField
                value={step.kind}
                onChange={(next) => changeStepKind(index, next as StepKind)}
                options={STEP_KINDS}
              />
              <span className="row-actions">
                <button
                  type="button"
                  className="btn btn-sm btn-quiet"
                  aria-label="上移"
                  disabled={index === 0}
                  onClick={() => moveStep(index, -1)}
                >
                  ↑
                </button>
                <button
                  type="button"
                  className="btn btn-sm btn-quiet"
                  aria-label="下移"
                  disabled={index === doc.steps.length - 1}
                  onClick={() => moveStep(index, 1)}
                >
                  ↓
                </button>
                <button
                  type="button"
                  className="btn btn-sm btn-quiet"
                  aria-label="删除步骤"
                  onClick={() => dropStep(index)}
                >
                  <IconTrash size={12} />
                </button>
              </span>
            </div>
            {step.kind === "llm" ? (
              <TextArea
                value={step.prompt ?? ""}
                onChange={(next) => patchStep(index, { prompt: next })}
                rows={4}
                mono
                placeholder={"标题：{{article.title}}\n正文：{{article.text}}"}
              />
            ) : null}
            {step.kind === "template" ? (
              <TextArea
                value={step.template ?? ""}
                onChange={(next) => patchStep(index, { template: next })}
                rows={3}
                mono
                placeholder="{{article.title}} — {{article.url}}"
              />
            ) : null}
            {step.kind === "http" ? (
              <div className="form-grid">
                <Field label="方法">
                  <SelectField
                    value={step.method ?? "POST"}
                    onChange={(next) => patchStep(index, { method: next })}
                    options={[
                      { value: "GET", label: "GET" },
                      { value: "POST", label: "POST" },
                      { value: "PUT", label: "PUT" },
                    ]}
                  />
                </Field>
                <Field label="地址" hint="模板变量同样可用，例如 {{article.url}}">
                  <TextField
                    value={step.url ?? ""}
                    onChange={(next) => patchStep(index, { url: next })}
                    placeholder="https://example.com/hook"
                    mono
                  />
                </Field>
                <Field label="请求体" hint="留空则发空包" wide>
                  <TextArea
                    value={step.body ?? ""}
                    onChange={(next) => patchStep(index, { body: next })}
                    rows={3}
                    mono
                  />
                </Field>
              </div>
            ) : null}
            {step.kind !== "template" ? (
              <Field label="覆盖模型" hint="留空用工作流默认模型">
                <SelectField
                  value={step.model ?? ""}
                  onChange={(next) => patchStep(index, { model: next || undefined })}
                  options={usable.map((model) => ({ value: model.slug, label: model.name }))}
                  placeholder="默认"
                />
              </Field>
            ) : null}
          </div>
        ))}
      </div>

      <div className="stack-sm">
        <div className="section-head">
          <span className="section-title">产出</span>
          <Switch
            checked={facet !== undefined}
            onChange={(next) => patch({ outputs: next ? { facet: { kind: "md", variant: "summary", body: "{{steps." + (doc.steps[0]?.id ?? "step1") + "}}" } } : {} })}
            label="写入内容分层"
          />
        </div>
        {facet ? (
          <div className="form-grid">
            <Field label="类型">
              <SelectField
                value={facet.kind}
                onChange={(next) => patch({ outputs: { facet: { ...facet, kind: next } } })}
                options={FACET_KINDS}
              />
            </Field>
            <Field label="变体" hint="同类型下的分栏名，例如 summary">
              <TextField
                value={facet.variant ?? ""}
                onChange={(next) => patch({ outputs: { facet: { ...facet, variant: next } } })}
                placeholder="summary"
                mono
              />
            </Field>
            <Field label="正文" hint="可引用文章字段与前面步骤的输出" wide>
              <TextArea
                value={facet.body}
                onChange={(next) => patch({ outputs: { facet: { ...facet, body: next } } })}
                rows={3}
                mono
              />
            </Field>
          </div>
        ) : (
          <p className="faint">不产出 facet：只做通知或外部请求。</p>
        )}
      </div>

      <div className="stack-sm">
        <div className="section-head">
          <span className="section-title">通知</span>
          <button type="button" className="btn btn-sm" onClick={addNotify}>
            <IconPlus size={12} /> 添加通知
          </button>
        </div>
        {notifies.length === 0 ? <p className="faint">默认不发通知；飞书机器人地址见「通知接入」页。</p> : null}
        {notifies.map((row, index) => (
          <div className="wf-notify" key={index}>
            <div className="wf-step-head">
              <SelectField
                value={row.kind}
                onChange={(next) => patchNotify(index, { kind: next as NotifyKind })}
                options={NOTIFY_KINDS}
              />
              <TextField
                value={row.url}
                onChange={(next) => patchNotify(index, { url: next })}
                placeholder={
                  row.kind === "feishu"
                    ? "https://open.feishu.cn/open-apis/bot/v2/hook/…"
                    : "https://example.com/webhook"
                }
                mono
              />
              <button
                type="button"
                className="btn btn-sm btn-quiet"
                aria-label="删除通知"
                onClick={() => dropNotify(index)}
              >
                <IconTrash size={12} />
              </button>
            </div>
            <div className="row">
              <label className="pick-inline">
                <input
                  type="checkbox"
                  checked={(row.on ?? []).includes("success")}
                  onChange={(event) =>
                    patchNotify(index, { on: toggleOn(row.on ?? [], "success", event.target.checked) })
                  }
                />
                <span className="faint">成功时</span>
              </label>
              <label className="pick-inline">
                <input
                  type="checkbox"
                  checked={(row.on ?? []).includes("failure")}
                  onChange={(event) =>
                    patchNotify(index, { on: toggleOn(row.on ?? [], "failure", event.target.checked) })
                  }
                />
                <span className="faint">失败时</span>
              </label>
              <TextField
                value={row.title ?? ""}
                onChange={(next) => patchNotify(index, { title: next })}
                placeholder="通知标题（可选）"
              />
            </div>
          </div>
        ))}
      </div>

      <div className="form-advanced">
        <button
          type="button"
          className="collapse-head"
          aria-expanded={advanced}
          onClick={() => setAdvanced(!advanced)}
        >
          <span className="collapse-title">高级 · 整份声明</span>
          <span className="collapse-note">headers、多段模板等表单没覆盖的字段在这里写</span>
        </button>
        {advanced ? (
          <div className="form-advanced-body">
            <TextArea value={draft} onChange={setDraft} rows={14} mono />
            <div className="row">
              <button type="button" className="btn btn-sm" onClick={applyDraft}>
                套用到表单
              </button>
              <button
                type="button"
                className="btn btn-sm btn-quiet"
                onClick={() => setDraft(JSON.stringify(doc, null, 2))}
              >
                用当前表单覆盖
              </button>
            </div>
            <p className="form-hint">`id` 与 `kind` 由系统在写入时补齐；保存前会重新解析。</p>
          </div>
        ) : null}
      </div>
    </Drawer>
  );
}

// ── 模型 ────────────────────────────────────────────────────────────────────

export function ModelDrawer({
  item,
  templates,
  onClose,
  onSaved,
}: {
  item: ModelRow | null;
  templates: ModelTemplate[];
  onClose: () => void;
  onSaved: (message: string) => void;
}) {
  const { notify } = useConsole();
  const editing = item !== null;
  const [slug, setSlug] = useState(item?.slug ?? "");
  const [doc, setDoc] = useState<ModelDoc>(() => item?.document ?? blankModel("api"));
  const [command, setCommand] = useState(() => (item?.document?.command ?? []).join(" "));
  const [advanced, setAdvanced] = useState(false);
  const [draft, setDraft] = useState(() => JSON.stringify(item?.document ?? blankModel("api"), null, 2));
  const [saving, setSaving] = useState(false);

  function patch(next: Partial<ModelDoc>) {
    setDoc((current) => ({ ...current, ...next }));
  }

  function changeMode(mode: ModelDoc["mode"]) {
    const base = blankModel(mode);
    setDoc((current) => ({
      ...base,
      name: current.name,
      description: current.description,
      system: current.system,
      timeout: current.timeout,
      mode,
    }));
    setCommand((base.command ?? []).join(" "));
  }

  function applyTemplate(template: ModelTemplate) {
    const next = template.doc;
    setDoc(next);
    setCommand((next.command ?? []).join(" "));
    setDraft(JSON.stringify(next, null, 2));
    notify("info", `已套用模板「${template.name}」`, "改好端点或命令后保存。");
  }

  function applyDraft() {
    try {
      const parsed = JSON.parse(draft) as ModelDoc;
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error("声明必须是对象");
      }
      setDoc(parsed);
      setCommand((parsed.command ?? []).join(" "));
      notify("ok", "已套用到表单", "保存时会再走一次完整校验。");
    } catch (problem) {
      notify("bad", "JSON 无法解析", problem instanceof Error ? problem.message : undefined);
    }
  }

  async function submit() {
    if (!slug.trim()) {
      notify("bad", "还缺模型名", "它同时是文件名，工作流里按这个名字引用。");
      return;
    }
    const payload: ModelDoc = { ...doc, command: doc.mode === "cli" ? splitCommand(command) : undefined };
    setSaving(true);
    try {
      const saved = await saveModel(slug.trim(), payload);
      onSaved(editing ? `已更新 ${saved.file}` : `已新建 ${saved.file}`);
      onClose();
    } catch (problem) {
      notify("bad", "没有通过校验", problem instanceof Error ? problem.message : undefined);
    } finally {
      setSaving(false);
    }
  }

  async function discard() {
    if (!item || !window.confirm(`删除 ${item.slug}？仍被工作流引用时会被校验拦下。`)) {
      return;
    }
    setSaving(true);
    try {
      await removeModel(item.slug);
      onSaved(`已删除 ${item.slug}`);
      onClose();
    } catch (problem) {
      notify("bad", "删除失败", problem instanceof Error ? problem.message : undefined);
    } finally {
      setSaving(false);
    }
  }

  const modeHint = MODEL_MODES.find((entry) => entry.value === doc.mode)?.hint ?? "";

  return (
    <Drawer
      open
      kicker={editing ? `模型 · ${item?.mode ?? doc.mode}` : "模型 · 新建"}
      title={editing ? item?.name ?? item?.slug ?? "" : "新建模型"}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn btn-primary" disabled={saving} onClick={() => void submit()}>
            {saving ? "写入中…" : editing ? "保存并校验" : "创建并校验"}
          </button>
          {editing ? (
            <button type="button" className="btn btn-quiet" disabled={saving} onClick={() => void discard()}>
              <IconTrash size={13} /> 删除
            </button>
          ) : null}
          <span className="spacer" />
          <button type="button" className="btn btn-quiet btn-sm" onClick={onClose}>
            取消
          </button>
        </>
      }
    >
      {!editing && templates.length ? (
        <div className="stack-sm">
          <span className="form-label">从模板起步</span>
          <div className="template-grid">
            {templates.map((template) => (
              <button
                key={template.slug}
                type="button"
                className="template-card"
                onClick={() => applyTemplate(template)}
              >
                <span className="template-name">{template.name}</span>
                <span className="template-desc">{template.description}</span>
                <span className="template-tag mono">model/{template.slug}</span>
              </button>
            ))}
          </div>
        </div>
      ) : null}

      <div className="form-grid">
        <Field label="模型名" hint={editing ? "新建后固定" : "工作流里按这个名字引用，例如 default"}>
          <TextField value={slug} onChange={setSlug} placeholder="default" mono disabled={editing} />
        </Field>
        <Field label="调用方式" hint={modeHint}>
          <Choice
            value={doc.mode}
            onChange={changeMode}
            options={MODEL_MODES.map((entry) => ({ value: entry.value, label: entry.label }))}
            label="模型调用方式"
          />
        </Field>
        <Field label="名称">
          <TextField value={doc.name ?? ""} onChange={(next) => patch({ name: next })} />
        </Field>
        <Field label="超时" hint="形如 90s、2m">
          <TextField value={doc.timeout ?? ""} onChange={(next) => patch({ timeout: next })} placeholder="90s" mono />
        </Field>

        {doc.mode === "cli" ? (
          <>
            <Field label="命令" hint="命令与参数用空格分隔；提示词写进子进程标准输入" wide>
              <TextField
                value={command}
                onChange={setCommand}
                placeholder="claude -p"
                mono
              />
            </Field>
            <Field label="系统提示" hint="可选，会拼在提示词前面" wide>
              <TextArea value={doc.system ?? ""} onChange={(next) => patch({ system: next })} rows={3} />
            </Field>
          </>
        ) : (
          <>
            <Field label="端点" hint="OpenAI 兼容的 /v1 地址">
              <TextField
                value={doc.endpoint ?? ""}
                onChange={(next) => patch({ endpoint: next })}
                placeholder="https://api.openai.com/v1"
                mono
              />
            </Field>
            <Field label="模型标识" hint="发给端点的 model 字段">
              <TextField value={doc.model ?? ""} onChange={(next) => patch({ model: next })} placeholder="gpt-4o-mini" mono />
            </Field>
            <Field
              label="密钥环境变量"
              hint={doc.mode === "api" ? "声明里只写变量名；启动采集进程时把密钥放进环境变量" : "本地端点通常不需要"}
            >
              <TextField
                value={doc.keyEnv ?? ""}
                onChange={(next) => patch({ keyEnv: next })}
                placeholder="OPENAI_API_KEY"
                mono
              />
            </Field>
            <Field label="温度" hint="0~2，越低越稳定">
              <NumberField
                value={doc.temperature ?? null}
                onChange={(next) => patch({ temperature: next ?? undefined })}
                placeholder="0.3"
              />
            </Field>
            <Field label="最大输出" hint="可选，单位 token">
              <NumberField
                value={doc.maxTokens ?? null}
                onChange={(next) => patch({ maxTokens: next ?? undefined })}
                placeholder="1024"
              />
            </Field>
            <Field label="系统提示" hint="可选，整段作为 system 消息" wide>
              <TextArea value={doc.system ?? ""} onChange={(next) => patch({ system: next })} rows={3} />
            </Field>
          </>
        )}
      </div>

      {doc.mode === "api" ? (
        <div className="note">
          <IconAlert size={14} />
          <span>
            密钥只从环境变量读，不写进声明文件。启动采集进程前设置好 <code className="mono">{doc.keyEnv || "OPENAI_API_KEY"}</code>，
            改完环境变量要重启进程。
          </span>
        </div>
      ) : null}

      <div className="form-advanced">
        <button
          type="button"
          className="collapse-head"
          aria-expanded={advanced}
          onClick={() => setAdvanced(!advanced)}
        >
          <span className="collapse-title">高级 · 整份声明</span>
          <span className="collapse-note">自定义 headers 等字段在这里补</span>
        </button>
        {advanced ? (
          <div className="form-advanced-body">
            <TextArea value={draft} onChange={setDraft} rows={12} mono />
            <div className="row">
              <button type="button" className="btn btn-sm" onClick={applyDraft}>
                套用到表单
              </button>
              <button
                type="button"
                className="btn btn-sm btn-quiet"
                onClick={() => setDraft(JSON.stringify(doc, null, 2))}
              >
                用当前表单覆盖
              </button>
            </div>
            <p className="form-hint">`id` 与 `kind` 由系统补齐；`keyEnv` 只是变量名。</p>
          </div>
        ) : null}
      </div>
    </Drawer>
  );
}

// ── 运行记录 ────────────────────────────────────────────────────────────────

export function RunDrawer({ run, onClose }: { run: WorkflowRun; onClose: () => void }) {
  return (
    <Drawer
      open
      kicker={`运行记录 · ${run.rid}`}
      title={run.name || run.workflow}
      onClose={onClose}
      footer={
        <>
          <span className="faint">
            {run.steps.length} 个步骤 · {run.durationMs} ms
          </span>
          <span className="spacer" />
          <button type="button" className="btn btn-sm btn-quiet" onClick={onClose}>
            关闭
          </button>
        </>
      }
    >
      <div className="row">
        <span className={`pill ${run.status === "ok" ? "pill-ok" : "pill-bad"}`}>{statusLabel(run.status)}</span>
        <span className="pill">{run.model}</span>
        {run.output ? <span className="pill pill-info">{run.output.kind}/{run.output.variant ?? "default"}</span> : null}
      </div>
      <dl className="defs">
        <dt>内容</dt>
        <dd className="mono">{run.rid}</dd>
        <dt>开始</dt>
        <dd>{run.startedAt ?? run.recordedAt ?? "—"}</dd>
        <dt>耗时</dt>
        <dd>{run.durationMs} ms</dd>
      </dl>
      {run.error ? (
        <div className="note is-bad">
          <IconAlert size={14} />
          <span>{run.error}</span>
        </div>
      ) : null}
      <div className="stack-sm">
        {run.steps.map((step) => (
          <div className="wf-run-step" key={step.id}>
            <div className="wf-step-head">
              <span className={`pill ${step.status === "ok" ? "pill-ok" : "pill-bad"}`}>{statusLabel(step.status)}</span>
              <span className="cell-strong">{step.id}</span>
              <span className="tag">{stepKindLabel(step.kind)}</span>
              <span className="faint mono">{step.ms} ms</span>
            </div>
            {step.error ? <p className="note is-bad">{step.error}</p> : null}
            {step.output ? <pre className="code">{step.output}</pre> : null}
          </div>
        ))}
      </div>
    </Drawer>
  );
}

// ── 小工具 ──────────────────────────────────────────────────────────────────

/** 切换步骤类型时清掉旧字段，避免一份步骤同时写着 prompt 与 url。 */
function seedOf(kind: StepKind): Partial<WorkflowStep> {
  if (kind === "llm") {
    return { prompt: "" };
  }
  if (kind === "template") {
    return { template: "" };
  }
  return { method: "POST", url: "", body: "" };
}

function toggleOn(current: ("success" | "failure")[], value: "success" | "failure", on: boolean): ("success" | "failure")[] {
  const next = on ? [...current, value] : current.filter((item) => item !== value);
  return Array.from(new Set(next));
}

/** 命令行按空白切分；带引号的参数留给高级 JSON 区。 */
function splitCommand(text: string): string[] {
  return text
    .split(/\s+/)
    .map((item) => item.trim())
    .filter(Boolean);
}
