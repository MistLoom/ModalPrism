const WRITE_HEADER = "x-modalprism-console";

export function readHeaders(): HeadersInit {
  return { Accept: "application/json" };
}

export function writeHeaders(): HeadersInit {
  return {
    Accept: "application/json",
    "Content-Type": "application/json",
    [WRITE_HEADER]: "1",
  };
}

export function formatLocalTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function querySuffix(asOf: number | null): string {
  return asOf != null ? `?asOfSnapshot=${asOf}` : "";
}
