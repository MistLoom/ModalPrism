export type ApiError = { error: { code: string; message: string } };

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`/console/api/v1${path}`, {
    headers: { Accept: "application/json", ...(init?.headers ?? {}) },
    ...init,
  });
  const body = await res.json().catch(() => ({}));
  if (!res.ok) {
    const err = body as ApiError;
    throw new Error(err.error?.message ?? res.statusText);
  }
  return body as T;
}

export const api = {
  meta: () => request<Record<string, unknown>>("/meta"),
  overview: () => request<Record<string, unknown>>("/overview"),
  sources: () => request<{ items: Record<string, unknown>[] }>("/sources"),
  source: (id: string) => request<Record<string, unknown>>(`/sources/${encodeURIComponent(id)}`),
  trigger: (id: string) =>
    request(`/sources/${encodeURIComponent(id)}/trigger`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ confirm: true }),
    }),
  patchSource: (id: string, enabled: boolean) =>
    request(`/sources/${encodeURIComponent(id)}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ enabled, confirm: true }),
    }),
  records: (limit = 40) => request<{ items: Record<string, unknown>[] }>(`/records?limit=${limit}`),
  governance: () => request<Record<string, unknown>>("/governance/egress"),
  resetCircuit: (host: string) =>
    request(`/governance/egress/${encodeURIComponent(host)}/reset-circuit`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ confirm: true }),
    }),
  configSummary: () => request<Record<string, unknown>>("/config/summary"),
  validateConfig: () =>
    request("/config/validate", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" }),
  reloadConfig: () =>
    request("/config/reload", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" }),
};
