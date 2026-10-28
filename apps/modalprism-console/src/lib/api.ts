import type {
  Attempt,
  Candidate,
  Channel,
  ContentDetail,
  ContentItem,
  Delivery,
  Draft,
  EffectiveSettings,
  FetchResult,
  HealthHistory,
  InputSet,
  ListResponse,
  LogRecord,
  ModelResource,
  NamedResource,
  Operation,
  Overview,
  Plan,
  PreconditionReport,
  ProductionRun,
  Project,
  ProjectResource,
  PromptResource,
  Publication,
  ResetPreview,
  SearchEngine,
  SearchResult,
  SettingsHistoryEntry,
  SettingsSchema,
  Snapshot,
  Source,
  SourceInput,
  SystemStatus,
  Task,
  WizardCheck,
  WizardDoc,
  Workflow,
  WorkflowStep,
  WorkflowVersion,
} from "./types";

export class ApiError extends Error {
  code: string;
  fieldErrors: { field: string; code: string; message: string }[];
  requestId: string;
  retryable: boolean;
  status: number;

  constructor(
    message: string,
    status: number,
    code = "internalError",
    requestId = "",
    retryable = false,
    fieldErrors: { field: string; code: string; message: string }[] = [],
  ) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    this.requestId = requestId;
    this.retryable = retryable;
    this.fieldErrors = fieldErrors;
  }
}

interface CallOptions {
  etag?: string;
  idempotencyKey?: string;
  noIdempotency?: boolean;
}

export interface Meta<T> {
  data: T;
  etag: string | null;
}

function key(): string {
  return crypto.randomUUID();
}

async function call<T>(method: string, path: string, body?: unknown, options: CallOptions = {}): Promise<T> {
  const { data } = await callWithMeta<T>(method, path, body, options);
  return data;
}

async function callWithMeta<T>(method: string, path: string, body?: unknown, options: CallOptions = {}): Promise<Meta<T>> {
  const headers: Record<string, string> = { accept: "application/json" };
  let payload: BodyInit | undefined;
  if (body instanceof FormData) {
    payload = body;
  } else if (body !== undefined) {
    headers["content-type"] = "application/json";
    payload = JSON.stringify(body);
  }
  if (!options.noIdempotency && method !== "GET") {
    headers["idempotency-key"] = options.idempotencyKey ?? key();
  }
  if (options.etag) {
    headers["if-match"] = options.etag;
  }
  let response: Response;
  try {
    response = await fetch(`/api/v1${path}`, { method, headers, body: payload });
  } catch {
    throw new ApiError("Node 控制台没有响应", 0, "dependencyUnavailable", "", true);
  }
  const text = await response.text();
  let parsed: unknown = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = null;
  }
  if (!response.ok) {
    const error = (parsed as { error?: { code?: string; message?: string; requestId?: string; retryable?: boolean; fieldErrors?: { field: string; code: string; message: string }[] } } | null)?.error;
    throw new ApiError(
      error?.message ?? `请求失败（${response.status}）`,
      response.status,
      error?.code ?? "internalError",
      error?.requestId ?? response.headers.get("x-request-id") ?? "",
      error?.retryable ?? false,
      error?.fieldErrors ?? [],
    );
  }
  return { data: parsed as T, etag: response.headers.get("etag") };
}

function query(values: Record<string, string | number | null | undefined>): string {
  const params = new URLSearchParams();
  for (const [name, value] of Object.entries(values)) {
    if (value !== undefined && value !== null && `${value}` !== "") params.set(name, `${value}`);
  }
  const text = params.toString();
  return text ? `?${text}` : "";
}

export async function waitOperation(operationId: string, onUpdate?: (operation: Operation) => void): Promise<Operation> {
  const started = Date.now();
  for (;;) {
    const operation = await call<Operation>("GET", `/operations/${operationId}`);
    onUpdate?.(operation);
    if (["succeeded", "partiallySucceeded", "failed", "cancelled"].includes(operation.status)) {
      return operation;
    }
    if (Date.now() - started > 10 * 60 * 1000) {
      throw new ApiError("操作等待超时", 0, "internalError", "", true);
    }
    await new Promise((resolve) => setTimeout(resolve, 600));
  }
}

export async function runOperation(
  operation: Operation,
  onUpdate?: (operation: Operation) => void,
): Promise<Operation> {
  const finished = await waitOperation(operation.operationId, onUpdate);
  if (finished.status === "failed") {
    throw new ApiError(
      finished.errorSummary?.message ?? "操作失败",
      0,
      finished.errorSummary?.code ?? "internalError",
      finished.errorSummary?.requestId ?? "",
      finished.retryable,
    );
  }
  return finished;
}

// system
export const systemStatus = () => call<SystemStatus>("GET", "/system/status");
export const systemOverview = () => call<Overview>("GET", "/system/overview");

// catalog
export const listSources = (params: { limit?: number; cursor?: string; query?: string; status?: string } = {}) =>
  call<ListResponse<Source>>("GET", `/catalog/sources${query(params)}`);
export const getSource = (id: string) => callWithMeta<Source>("GET", `/catalog/sources/${encodeURIComponent(id)}`);
export const createSource = (source: SourceInput) => call<Source>("POST", "/catalog/sources", source);
export const updateSource = (id: string, source: SourceInput, etag: string) =>
  call<Source>("PATCH", `/catalog/sources/${encodeURIComponent(id)}`, source, { etag });
export const deleteSource = (id: string, etag: string) =>
  call<Operation>("DELETE", `/catalog/sources/${encodeURIComponent(id)}`, undefined, { etag });
export const enableSource = (id: string, etag: string) =>
  call<Source>("POST", `/catalog/sources/${encodeURIComponent(id)}/enable`, {}, { etag });
export const disableSource = (id: string, etag: string) =>
  call<Source>("POST", `/catalog/sources/${encodeURIComponent(id)}/disable`, {}, { etag });
export const probeSource = (id: string, locator?: string) =>
  call<Operation>("POST", `/catalog/sources/${encodeURIComponent(id)}/probe`, { locator, persistResult: false });
export const listAccessMethods = () => call<ListResponse<NamedResource>>("GET", "/catalog/accessMethods?limit=200");
export const listAdapters = () => call<ListResponse<NamedResource>>("GET", "/catalog/adapters?limit=200");
export const listTemplates = () => call<ListResponse<NamedResource>>("GET", "/catalog/templates?limit=200");
export const analyzeSource = (input: { url: string; instruction?: string; modelId?: string }) =>
  call<Operation>("POST", "/catalog/sourceAnalyses", input);
export const listAdapterDrafts = () => call<ListResponse<NamedResource>>("GET", "/catalog/adapterDrafts?limit=200");
export const createAdapterDraft = (input: { sourceAnalysisId: string; name: string; instruction?: string }) =>
  call<Operation>("POST", "/catalog/adapterDrafts", input);
export const approveAdapterDraft = (id: string, etag: string, comment: string) =>
  call<Operation>("POST", `/catalog/adapterDrafts/${encodeURIComponent(id)}/approve`, { validationId: id, comment }, { etag });
export const rejectAdapterDraft = (id: string, etag: string, comment: string) =>
  call<NamedResource>("POST", `/catalog/adapterDrafts/${encodeURIComponent(id)}/reject`, { decision: "reject", comment }, { etag });

// collection
export const listPlans = (params: { limit?: number; cursor?: string; status?: string } = {}) =>
  call<ListResponse<Plan>>("GET", `/collection/plans${query(params)}`);
export const updatePlan = (id: string, schedule: Plan["schedule"], etag: string) =>
  call<Plan>("PATCH", `/collection/plans/${encodeURIComponent(id)}`, { schedule }, { etag });
export const pausePlan = (id: string, etag: string) =>
  call<Plan>("POST", `/collection/plans/${encodeURIComponent(id)}/pause`, {}, { etag });
export const resumePlan = (id: string, etag: string) =>
  call<Plan>("POST", `/collection/plans/${encodeURIComponent(id)}/resume`, {}, { etag });
export const createCollectionRun = (input: { sourceId: string; mode: string; locator?: string; reason?: string }) =>
  call<Operation>("POST", "/collection/runs", input);
export const listCollectionRuns = (params: { limit?: number; cursor?: string; status?: string; sourceId?: string } = {}) =>
  call<ListResponse<Record<string, unknown>>>("GET", `/collection/runs${query(params)}`);

// content
export const listContent = (params: { limit?: number; cursor?: string; query?: string; sourceId?: string; status?: string } = {}) =>
  call<ListResponse<ContentItem>>("GET", `/content/items${query(params)}`);
export const getContent = (id: string) => callWithMeta<ContentDetail>("GET", `/content/items/${encodeURIComponent(id)}`);
export const patchContent = (id: string, patch: { tags?: string[]; title?: string }, etag: string) =>
  call<ContentItem>("PATCH", `/content/items/${encodeURIComponent(id)}`, patch, { etag });
export const deleteContent = (id: string, etag: string) =>
  call<Operation>("DELETE", `/content/items/${encodeURIComponent(id)}`, undefined, { etag });
export const batchContent = (input: { ids: string[]; action: string; tags?: string[] }) =>
  call<Operation>("POST", "/content/items/batch", input);
export const contentPurge = (input: { source?: string; before?: string; after?: string; dryRun?: boolean }) =>
  call<{ matched: number; deleted: number; truncated: boolean; dryRun: boolean }>("POST", "/content/items/purge", input);
export const listContentVersions = (id: string, params: { limit?: number; cursor?: string } = {}) =>
  call<ListResponse<Record<string, unknown>>>("GET", `/content/items/${encodeURIComponent(id)}/versions${query(params)}`);
export const contentLineage = (id: string) =>
  call<{ nodes: Record<string, unknown>[]; edges: Record<string, unknown>[] }>("GET", `/content/items/${encodeURIComponent(id)}/lineage`);
export const contentRelations = (id: string) =>
  call<ListResponse<Record<string, unknown>>>("GET", `/content/items/${encodeURIComponent(id)}/relations?limit=200`);
export const listSnapshots = (params: { limit?: number; cursor?: string } = {}) =>
  call<ListResponse<Snapshot>>("GET", `/content/snapshots${query(params)}`);
export const replaySnapshot = (input: { snapshotId: string; scope: string; contentIds?: string[] }) =>
  call<Operation>("POST", "/content/replays", input);

// production
export const listInputSets = (params: { limit?: number; cursor?: string } = {}) =>
  call<ListResponse<InputSet>>("GET", `/production/inputSets${query(params)}`);
export const createInputSet = (input: { name: string; inputs: Record<string, unknown>[] }) =>
  call<InputSet>("POST", "/production/inputSets", input);
export const uploadInputFile = (file: File) => {
  const form = new FormData();
  form.append("file", file);
  form.append("name", file.name);
  return call<Record<string, unknown>>("POST", "/production/inputSets/upload", form);
};
export const listWorkflows = (params: { limit?: number; cursor?: string } = {}) =>
  call<ListResponse<Workflow>>("GET", `/production/workflows${query(params)}`);
export const createWorkflow = (input: { name: string; description: string }) =>
  call<Workflow>("POST", "/production/workflows", input);
export const listWorkflowVersions = (id: string) =>
  call<ListResponse<WorkflowVersion>>("GET", `/production/workflows/${encodeURIComponent(id)}/versions?limit=200`);
export const createWorkflowVersion = (id: string, steps: WorkflowStep[], etag: string) =>
  call<WorkflowVersion>("POST", `/production/workflows/${encodeURIComponent(id)}/versions`, { steps }, { etag });
export const listProductionRuns = (params: { limit?: number; cursor?: string; status?: string } = {}) =>
  call<ListResponse<ProductionRun>>("GET", `/production/runs${query(params)}`);
export const createProductionRun = (input: { inputSetId: string; workflowVersionId: string; mode?: string }) =>
  call<Operation>("POST", "/production/runs", input);
export const getProductionRun = (id: string) => call<ProductionRun>("GET", `/production/runs/${encodeURIComponent(id)}`);
export const listRunAttempts = (id: string) =>
  call<ListResponse<Attempt>>("GET", `/production/runs/${encodeURIComponent(id)}/attempts?limit=200`);
export const retryProductionRun = (id: string, reason?: string) =>
  call<Operation>("POST", `/production/runs/${encodeURIComponent(id)}/retry`, { reason });
export const listCandidates = (params: { limit?: number; cursor?: string; status?: string } = {}) =>
  call<ListResponse<Candidate>>("GET", `/production/candidates${query(params)}`);
export const reviewCandidate = (id: string, etag: string, input: { decision: string; comment?: string; selectedOutputIds?: string[] }) =>
  call<Record<string, unknown>>("POST", `/production/candidates/${encodeURIComponent(id)}/reviews`, input, { etag });
export const listDrafts = (params: { limit?: number; cursor?: string } = {}) =>
  call<ListResponse<Draft>>("GET", `/production/drafts${query(params)}`);
export const createDraft = (input: { candidateIds: string[]; title: string; body: string }) =>
  call<Draft>("POST", "/production/drafts", input);
export const getDraft = (id: string) => callWithMeta<Draft>("GET", `/production/drafts/${encodeURIComponent(id)}`);
export const updateDraft = (id: string, input: { candidateIds: string[]; title: string; body: string }, etag: string) =>
  call<Draft>("PATCH", `/production/drafts/${encodeURIComponent(id)}`, input, { etag });

// publication
export const listChannels = () => call<ListResponse<Channel>>("GET", "/publication/channels?limit=200");
export const createChannel = (input: { name: string; kind: string; enabled: boolean; settings: Record<string, unknown> }) =>
  call<Channel>("POST", "/publication/channels", input);
export const updateChannel = (id: string, input: { name: string; kind: string; enabled: boolean; settings: Record<string, unknown> }, etag: string) =>
  call<Channel>("PATCH", `/publication/channels/${encodeURIComponent(id)}`, input, { etag });
export const deleteChannel = (id: string, etag: string) =>
  call<Record<string, unknown>>("DELETE", `/publication/channels/${encodeURIComponent(id)}`, undefined, { etag });
export const testNotification = (channelId: string) =>
  call<Operation>("POST", "/publication/notifications/test", { channelId });
export const listPublications = (params: { limit?: number; cursor?: string; status?: string } = {}) =>
  call<ListResponse<Publication>>("GET", `/publications${query(params)}`);
export const createPublication = (input: { draftId: string; channelIds: string[]; scheduledAt?: string | null }) =>
  call<Operation>("POST", "/publications", input);
export const getPublication = (id: string) => call<Publication>("GET", `/publications/${encodeURIComponent(id)}`);
export const listDeliveries = (params: { limit?: number; cursor?: string; status?: string } = {}) =>
  call<ListResponse<Delivery>>("GET", `/publication/deliveries${query(params)}`);
export const retryDelivery = (id: string, reason?: string) =>
  call<Operation>("POST", `/publication/deliveries/${encodeURIComponent(id)}/retry`, { reason });
export const egressStatus = () => call<Record<string, unknown>>("GET", "/publication/egressStatus");
export const resetEgressHost = (host: string) =>
  call<Operation>("POST", `/publication/egressHosts/${encodeURIComponent(host)}/reset`, {});

// settings
export const getSettingsSchema = () => call<SettingsSchema>("GET", "/settings/schema");
export const getEffectiveSettings = () => callWithMeta<EffectiveSettings>("GET", "/settings/effective");
export const validateSettings = (changes: Record<string, unknown>) =>
  call<Record<string, unknown>>("POST", "/settings/validate", { changes });
export const diffSettings = (changes: Record<string, unknown>) =>
  call<Record<string, unknown>>("POST", "/settings/diff", { changes });
export const applySettings = (changes: Record<string, unknown>, reason: string, etag: string) =>
  call<Operation>("POST", "/settings/apply", { changes, reason }, { etag });
export const settingsHistory = (params: { limit?: number; cursor?: string } = {}) =>
  call<ListResponse<SettingsHistoryEntry>>("GET", `/settings/history${query(params)}`);
export const rollbackSettings = (targetGeneration: number, reason: string, etag: string) =>
  call<Operation>("POST", "/settings/rollback", { targetGeneration, reason }, { etag });
export const listModels = () => call<ListResponse<ModelResource>>("GET", "/settings/models?limit=200");
export const createModel = (input: Record<string, unknown>) =>
  call<ModelResource>("POST", "/settings/models", input);
export const listPrompts = () => call<ListResponse<PromptResource>>("GET", "/settings/prompts?limit=200");
export const createPrompt = (input: { name: string; content: string; description?: string }) =>
  call<PromptResource>("POST", "/settings/prompts", input);
export const createPromptVersion = (id: string, etag: string, input: { name?: string; content: string; description?: string }) =>
  call<Record<string, unknown>>("POST", `/settings/prompts/${encodeURIComponent(id)}/versions`, input, { etag });
export const listSkills = () => call<ListResponse<PromptResource>>("GET", "/settings/skills?limit=200");
export const createSkill = (input: { name: string; content: string; description?: string }) =>
  call<PromptResource>("POST", "/settings/skills", input);
export const createSkillVersion = (id: string, etag: string, input: { name?: string; content: string; description?: string }) =>
  call<Record<string, unknown>>("POST", `/settings/skills/${encodeURIComponent(id)}/versions`, input, { etag });

// operations
export const listOperations = (params: { limit?: number; cursor?: string; status?: string } = {}) =>
  call<ListResponse<Operation>>("GET", `/operations${query(params)}`);
export const getOperation = (id: string) => call<Operation>("GET", `/operations/${encodeURIComponent(id)}`);
export const operationEvents = (id: string, params: { limit?: number; cursor?: string } = {}) =>
  call<ListResponse<Record<string, unknown>>>("GET", `/operations/${encodeURIComponent(id)}/events${query(params)}`);
export const cancelOperation = (id: string) => call<Operation>("POST", `/operations/${encodeURIComponent(id)}/cancel`, {});
export const retryOperation = (id: string, reason?: string) =>
  call<Operation>("POST", `/operations/${encodeURIComponent(id)}/retry`, { reason });
export const listTasks = (params: { limit?: number; cursor?: string; status?: string; kind?: string; sourceId?: string } = {}) =>
  call<ListResponse<Task>>("GET", `/operations/tasks${query(params)}`);
export const batchTasks = (ids: string[], action: string) =>
  call<Operation>("POST", "/operations/tasks/batch", { ids, action });
export const purgeTasks = (input: { status?: string; kind?: string; sourceId?: string; before?: string; dryRun?: boolean }) =>
  call<{ matched: number; deleted: number; dryRun: boolean }>("POST", "/operations/tasks/purge", input);
export const listQueues = () => call<ListResponse<Record<string, unknown>>>("GET", "/operations/queues");
export const getHealth = () => call<Record<string, unknown>>("GET", "/operations/health");
export const listHealthHistory = (params: { limit?: number; cursor?: string; sourceId?: string } = {}) =>
  call<ListResponse<HealthHistory>>("GET", `/operations/healthHistory${query(params)}`);
export const recomputeHealth = () => call<Operation>("POST", "/operations/healthRecomputations", {});

// observability
export const listLogs = (params: Record<string, string | number | null | undefined> = {}) =>
  call<ListResponse<LogRecord>>("GET", `/observability/logs${query(params)}`);
export const listEvents = (params: Record<string, string | number | null | undefined> = {}) =>
  call<ListResponse<LogRecord>>("GET", `/observability/events${query(params)}`);
export const listAuditEvents = (params: Record<string, string | number | null | undefined> = {}) =>
  call<ListResponse<LogRecord>>("GET", `/observability/auditEvents${query(params)}`);
export const exportObservability = (stream: string, format: string, filters: Record<string, unknown>) =>
  call<Operation>("POST", "/observability/exports", { stream, format, filters });

// maintenance
export const previewReset = (domains: string[]) =>
  callWithMeta<ResetPreview>("POST", "/maintenance/resetPreviews", { domains });
export const createReset = (preview: ResetPreview, token: string) =>
  call<Operation>("POST", "/maintenance/resets", { previewId: preview.id, confirmationToken: token }, { etag: preview.etag });
export const previewFactoryReset = () =>
  callWithMeta<ResetPreview>("POST", "/maintenance/factoryResetPreviews", {});
export const createFactoryReset = (preview: ResetPreview, token: string) =>
  call<Operation>("POST", "/maintenance/factoryResets", { previewId: preview.id, confirmationToken: token }, { etag: preview.etag });
export const getIntegrityReport = (id: string) =>
  call<Record<string, unknown>>("GET", `/maintenance/integrityReports/${encodeURIComponent(id)}`);

// projects
export const listProjects = () => call<ListResponse<Project>>("GET", "/projects");
export const createProject = (input: { name: string; description?: string }) =>
  call<Project>("POST", "/projects", input);
export const listProjectResources = (id: string, kind?: string) => {
  const params = { kind };
  return call<ListResponse<ProjectResource>>(
    "GET",
    `/projects/${encodeURIComponent(id)}/resources${query(params)}`,
  );
};
export const bindProjectResource = (id: string, kind: string, resourceId: string) =>
  call<void>("POST", `/projects/${encodeURIComponent(id)}/resources`, { kind, id: resourceId }, { noIdempotency: true });
export const unbindProjectResource = (id: string, kind: string, resourceId: string) =>
  call<void>("DELETE", `/projects/${encodeURIComponent(id)}/resources/${encodeURIComponent(kind)}/${encodeURIComponent(resourceId)}`, undefined, { noIdempotency: true });

// wizard
export const getWizard = (id: string) => call<WizardDoc>("GET", `/projects/${encodeURIComponent(id)}/wizard`);
export const createWizard = (id: string) =>
  call<WizardDoc>("POST", `/projects/${encodeURIComponent(id)}/wizard`, {}, { noIdempotency: true });
export const checkWizardStep = (id: string, stepId: string) =>
  call<WizardCheck>("POST", `/projects/${encodeURIComponent(id)}/wizard/steps/${encodeURIComponent(stepId)}/check`, {}, { noIdempotency: true });
export const completeWizardStep = (id: string, stepId: string) =>
  call<WizardDoc>("POST", `/projects/${encodeURIComponent(id)}/wizard/steps/${encodeURIComponent(stepId)}/complete`, {}, { noIdempotency: true });
export const skipWizardStep = (id: string, stepId: string) =>
  call<WizardDoc>("POST", `/projects/${encodeURIComponent(id)}/wizard/steps/${encodeURIComponent(stepId)}/skip`, {}, { noIdempotency: true });
export const dismissWizard = (id: string) =>
  call<WizardDoc>("POST", `/projects/${encodeURIComponent(id)}/wizard/dismiss`, {}, { noIdempotency: true });

// search / surface
export const listSearchEngines = () => call<ListResponse<SearchEngine>>("GET", "/search/engines");
export const runSearch = (input: { query: string; engine?: string; projectId?: string; limit?: number }) =>
  call<SearchResult>("POST", "/search", input, { noIdempotency: true });
export const fetchUrl = (url: string, maxBytes?: number) =>
  call<FetchResult>("POST", "/fetch", { url, maxBytes }, { noIdempotency: true });
export const parseInput = (input: { text?: string; url?: string; selectors?: string[] }) =>
  call<Record<string, unknown>>("POST", "/parse", input, { noIdempotency: true });
export const getPreconditions = (scope?: string) => {
  const params = { scope };
  return call<PreconditionReport>("GET", `/preconditions${query(params)}`);
};
