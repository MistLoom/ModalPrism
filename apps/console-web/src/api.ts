import { querySuffix, readHeaders, writeHeaders } from "./format";

export type ApiError = { error: { code: string; message: string } };

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`/console/api/v1${path}`, {
    ...init,
    headers: {
      ...(init?.method && init.method !== "GET" ? writeHeaders() : readHeaders()),
      ...(init?.headers ?? {}),
    },
  });
  const body = await res.json().catch(() => ({}));
  if (!res.ok) {
    const err = body as ApiError;
    throw new Error(err.error?.message ?? res.statusText);
  }
  return body as T;
}

export const api = {
  meta: (asOf: number | null) => request<Record<string, unknown>>(`/meta${querySuffix(asOf)}`),
  timeline: () => request<{ items: Record<string, unknown>[] }>("/timeline"),
  overview: () => request<Record<string, unknown>>("/overview"),
  sources: () =>
    request<{
      items: Record<string, unknown>[];
      readOnly?: boolean;
      viewNote?: string;
      historicalMode?: boolean;
    }>("/sources"),
  trigger: (id: string) =>
    request(`/sources/${encodeURIComponent(id)}/trigger`, {
      method: "POST",
      body: JSON.stringify({ confirm: true }),
    }),
  patchSource: (id: string, enabled: boolean) =>
    request(`/sources/${encodeURIComponent(id)}`, {
      method: "PATCH",
      body: JSON.stringify({ enabled, confirm: true }),
    }),
  records: (opts: {
    limit?: number;
    cursor?: string;
    source?: string;
    q?: string;
    asOf?: number | null;
  }) => {
    const p = new URLSearchParams();
    if (opts.limit) p.set("limit", String(opts.limit));
    if (opts.cursor) p.set("cursor", opts.cursor);
    if (opts.source) p.set("source", opts.source);
    if (opts.q) p.set("q", opts.q);
    if (opts.asOf != null) p.set("asOfSnapshot", String(opts.asOf));
    const q = p.toString();
    return request<{
      items: Record<string, unknown>[];
      nextCursor?: string;
      readOnly?: boolean;
      sourceOptions?: { id: string; name: string }[];
    }>(`/records${q ? `?${q}` : ""}`);
  },
  record: (id: string, asOf: number | null) =>
    request<Record<string, unknown>>(`/records/${encodeURIComponent(id)}${querySuffix(asOf)}`),
  governance: () =>
    request<Record<string, unknown> & { readOnly?: boolean; viewNote?: string }>(
      "/governance/egress",
    ),
  resetCircuit: (host: string) =>
    request(`/governance/egress/${encodeURIComponent(host)}/reset-circuit`, {
      method: "POST",
      body: JSON.stringify({ confirm: true }),
    }),
  configSummary: () => request<Record<string, unknown>>("/config/summary"),
  configFiles: () =>
    request<{
      items: string[];
      groups: { id: string; label: string; items: { path: string; label: string }[] }[];
    }>("/config/files"),
  configFile: (path: string) =>
    request<{ path: string; text: string; canRollback: boolean }>(
      `/config/files/${encodeURIComponent(path)}`,
    ),
  validateFileText: (path: string, text: string) =>
    request("/config/file-validate", {
      method: "POST",
      body: JSON.stringify({ path, text }),
    }),
  putConfigFile: (path: string, text: string) =>
    request(`/config/files/${encodeURIComponent(path)}`, {
      method: "PUT",
      body: JSON.stringify({ text, confirm: true }),
    }),
  rollbackConfigFile: (path: string) =>
    request("/config/file-rollback", {
      method: "POST",
      body: JSON.stringify({ path, confirm: true }),
    }),
  validateConfig: () => request("/config/validate", { method: "POST", body: "{}" }),
  reloadConfig: () => request("/config/reload", { method: "POST", body: "{}" }),
};
