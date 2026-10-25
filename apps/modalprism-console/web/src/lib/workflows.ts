// 内容加工的界面入口：工作流、模型、运行记录与通知。
//
// 声明形状与采集进程里的 `WorkflowSpec` / `ModelSpec` 一一对应（camelCase），
// 字段校验留在服务端：这里只负责组装草稿、给出人读的名字，不预判合法性。

import { call } from "./api";

export type ModelMode = "api" | "local" | "cli";
export type StepKind = "llm" | "template" | "http";
export type NotifyKind = "webhook" | "feishu";
export type NotifyOn = "success" | "failure";
export type TriggerOn = "commit" | "manual";

export interface WorkflowStep {
  id: string;
  kind: StepKind;
  prompt?: string;
  template?: string;
  method?: string;
  url?: string;
  headers?: [string, string][];
  body?: string;
  model?: string;
}

export interface WorkflowNotify {
  kind: NotifyKind;
  url: string;
  on?: NotifyOn[];
  title?: string;
}

export interface WorkflowDoc {
  name?: string;
  description?: string;
  enabled?: boolean;
  trigger?: { on?: TriggerOn; sources?: string[]; kinds?: string[] };
  model: string;
  steps: WorkflowStep[];
  outputs?: { facet?: { kind: string; variant?: string; body: string } };
  notify?: WorkflowNotify[];
}

export interface ModelDoc {
  name?: string;
  description?: string;
  mode: ModelMode;
  endpoint?: string;
  model?: string;
  keyEnv?: string;
  headers?: [string, string][];
  command?: string[];
  system?: string;
  timeout?: string;
  maxTokens?: number;
  temperature?: number;
}

/** 列表行：摘要 + 可回填的正文。 */
export interface WorkflowRow {
  slug: string;
  file?: string;
  name: string;
  description?: string | null;
  enabled: boolean;
  available?: boolean;
  trigger?: { on?: string; sources?: string[]; kinds?: string[] };
  model?: string;
  steps?: { id: string; kind: string }[];
  notify?: { kind: string; on?: string[] }[];
  outputs?: { kind: string; variant?: string | null } | null;
  error?: string;
  document?: WorkflowDoc;
}

export interface ModelRow {
  slug: string;
  name: string;
  mode: ModelMode;
  model?: string | null;
  endpoint?: string | null;
  keyEnv?: string | null;
  command?: string[];
  error?: string;
  document?: ModelDoc;
}

export interface WorkflowTemplate {
  slug: string;
  name: string;
  description: string;
  doc: WorkflowDoc;
}

export interface ModelTemplate {
  slug: string;
  name: string;
  description: string;
  doc: ModelDoc;
}

export interface WorkflowCatalog {
  workflows: WorkflowRow[];
  models: ModelRow[];
  templates: { workflows: WorkflowTemplate[]; models: ModelTemplate[] };
}

export interface RunStep {
  id: string;
  kind: string;
  status: "ok" | "failed" | string;
  ms: number;
  output?: string | null;
  error?: string | null;
}

export interface WorkflowRun {
  workflow: string;
  name: string;
  status: "ok" | "failed" | string;
  rid: string;
  seq: number;
  model: string;
  startedAt?: string | null;
  durationMs: number;
  steps: RunStep[];
  error?: string | null;
  output?: { kind: string; variant?: string | null } | null;
  runRid?: string;
  recordedAt?: string | null;
}

export interface TickResult {
  cursor?: number;
  next?: number;
  scanned?: number;
  processed?: number;
  failed?: number;
  runs?: WorkflowRun[];
}

export async function catalog(): Promise<WorkflowCatalog> {
  const value = await call<Partial<WorkflowCatalog> | null>("workflow", "catalog");
  return {
    workflows: value?.workflows ?? [],
    models: value?.models ?? [],
    templates: {
      workflows: value?.templates?.workflows ?? [],
      models: value?.templates?.models ?? [],
    },
  };
}

export interface WorkflowDetail {
  slug: string;
  file: string;
  document: WorkflowDoc;
  available: boolean;
}

export async function get(slug: string): Promise<WorkflowDetail> {
  return call<WorkflowDetail>("workflow", "get", { slug });
}

export async function save(slug: string, doc: WorkflowDoc): Promise<{ slug: string; file: string }> {
  return call("workflow", "save", { slug, doc });
}

export async function remove(slug: string): Promise<unknown> {
  return call("workflow", "remove", { slug });
}

export async function toggle(slug: string, enabled: boolean): Promise<unknown> {
  return call("workflow", "toggle", { slug, enabled });
}

export async function saveModel(slug: string, doc: ModelDoc): Promise<{ slug: string; file: string }> {
  return call("workflow", "save_model", { slug, doc });
}

export async function removeModel(slug: string): Promise<unknown> {
  return call("workflow", "remove_model", { slug });
}

/** 手动把一条已入库的内容过一遍工作流。 */
export async function run(slug: string, rid: string): Promise<WorkflowRun> {
  return call<WorkflowRun>("workflow", "run", { slug, rid });
}

export async function runs(limit = 20): Promise<WorkflowRun[]> {
  return (await call<WorkflowRun[] | null>("workflow", "runs", { limit })) ?? [];
}

/** 处理游标之后的新内容；界面用它代替等待下一轮。 */
export async function tick(): Promise<TickResult> {
  return call<TickResult>("workflow", "tick");
}

export async function notifyTest(input: {
  kind: NotifyKind;
  url: string;
  title?: string;
}): Promise<{ ok?: boolean }> {
  return call("workflow", "notify_test", input);
}

// ── 人读的名字 ──────────────────────────────────────────────────────────────

export const MODEL_MODES: { value: ModelMode; label: string; hint: string }[] = [
  { value: "api", label: "远端 API", hint: "OpenAI 兼容的 chat completions，密钥从环境变量读" },
  { value: "local", label: "本地模型", hint: "Ollama、vLLM、LM Studio 等本机端点" },
  { value: "cli", label: "命令行", hint: "提示词写进子进程标准输入，取标准输出" },
];

export const STEP_KINDS: { value: StepKind; label: string }[] = [
  { value: "llm", label: "模型生成" },
  { value: "template", label: "模板拼接" },
  { value: "http", label: "外部请求" },
];

export const TRIGGER_ONS: { value: TriggerOn; label: string }[] = [
  { value: "commit", label: "新内容入库时" },
  { value: "manual", label: "只在手动运行" },
];

export const NOTIFY_KINDS: { value: NotifyKind; label: string }[] = [
  { value: "webhook", label: "通用 Webhook" },
  { value: "feishu", label: "飞书机器人" },
];

export function modeLabel(mode: string): string {
  return MODEL_MODES.find((item) => item.value === mode)?.label ?? mode;
}

export function stepKindLabel(kind: string): string {
  return STEP_KINDS.find((item) => item.value === kind)?.label ?? kind;
}

export function triggerLabel(on: string | undefined): string {
  return TRIGGER_ONS.find((item) => item.value === on)?.label ?? "新内容入库时";
}

export function notifyLabel(kind: string): string {
  return NOTIFY_KINDS.find((item) => item.value === kind)?.label ?? kind;
}

export function statusLabel(status: string): string {
  return status === "ok" ? "成功" : status === "failed" ? "失败" : status;
}

/** 模型声明的终点：API 用端点与模型名，命令行用命令。 */
export function modelTarget(model: ModelRow): string {
  if (model.error) {
    return "声明无效";
  }
  if (model.mode === "cli") {
    return (model.command ?? []).join(" ") || "未配置命令";
  }
  const endpoint = (model.endpoint ?? "").replace(/^https?:\/\//, "");
  return model.model ? `${model.model} · ${endpoint}` : endpoint || "未配置端点";
}

/** 空白草稿。新建时从模板起步，这里是「不要模板」的那条路。 */
export function blankWorkflow(model: string): WorkflowDoc {
  return {
    name: "新的工作流",
    description: "",
    enabled: true,
    trigger: { on: "commit", kinds: ["article"], sources: [] },
    model,
    steps: [{ id: "step1", kind: "llm", prompt: "标题：{{article.title}}\n正文：{{article.text}}\n" }],
    outputs: { facet: { kind: "md", variant: "summary", body: "{{steps.step1}}" } },
    notify: [],
  };
}

export function blankStep(index: number): WorkflowStep {
  return { id: `step${index}`, kind: "llm", prompt: "" };
}

export function blankModel(mode: ModelMode): ModelDoc {
  if (mode === "cli") {
    return { name: "命令行模型", mode, command: ["claude", "-p"], timeout: "180s" };
  }
  if (mode === "local") {
    return {
      name: "本地模型",
      mode,
      endpoint: "http://127.0.0.1:11434/v1",
      model: "qwen2.5:7b",
      timeout: "120s",
    };
  }
  return {
    name: "远端模型",
    mode,
    endpoint: "https://api.openai.com/v1",
    model: "gpt-4o-mini",
    keyEnv: "OPENAI_API_KEY",
    timeout: "90s",
  };
}

/** 逗号 / 空白分隔的输入换成数组；空输入是空数组。 */
export function splitList(value: string): string[] {
  return value
    .split(/[,，\s]+/)
    .map((item) => item.trim())
    .filter(Boolean);
}

export function joinList(value: string[] | undefined): string {
  return (value ?? []).join(", ");
}
