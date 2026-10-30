#!/usr/bin/env node
// ModalPrism 公共 HTTP 的薄封装：一个命令一件事，输出原样 JSON。
// 只用 Node 18+ 自带的 fetch，无依赖；失败时退出码 1，正文打到 stderr。

const DEFAULT_BASE = "http://127.0.0.1:17880";

const args = process.argv.slice(2);
const command = args.shift() ?? "help";
const flags = {};
const rest = [];
for (let index = 0; index < args.length; index += 1) {
  const item = args[index];
  if (!item.startsWith("--")) {
    rest.push(item);
    continue;
  }
  const key = item.slice(2);
  const next = args[index + 1];
  if (next === undefined || next.startsWith("--")) {
    flags[key] = key === "project" ? "" : "true";
  } else {
    flags[key] = next;
    index += 1;
  }
}

const base = (flags.base ?? process.env.MODALPRISM_BASE ?? DEFAULT_BASE).replace(/\/+$/, "");
const project = flags.project?.trim();

const usage = `用法: node prism.mjs <命令> [参数] [--base http://host:port] [--project projectId]

  --project projectId                       指定项目；省略时沿用服务端默认项目

  status                                    系统状态与配置代际
  content [--query Q] [--source ID] [--status active|tombstoned] [--limit N]
  content-item <contentId>                  内容详情与正文
  sources [--limit N]
  targets                                   投递目标：模型、外部 API、通知渠道
  records [--type manual|auto] [--status S] [--limit N]
  deliver <contentId>[,<contentId>…] --targets kind:id[,kind:id…] [--note N]
  op <operationId>                          Operation 状态
  logs [--correlation ID] [--level L] [--limit N]
`;

function newKey() {
  return `skill-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

async function request(path, init) {
  const headers = new Headers(init?.headers);
  if (project) headers.set("x-project-id", project);
  const response = await fetch(`${base}${path}`, { ...init, headers });
  const body = await response.text();
  let data;
  try {
    data = body ? JSON.parse(body) : null;
  } catch {
    throw new Error(`返回的不是 JSON: ${body.slice(0, 200)}`);
  }
  if (!response.ok) {
    const error = data?.error;
    throw new Error(`${response.status} ${error?.code ?? "error"}: ${error?.message ?? body.slice(0, 200)}`);
  }
  return data;
}

function query(pairs) {
  const search = new URLSearchParams();
  for (const [key, value] of pairs) {
    if (value !== undefined) search.set(key, value);
  }
  const text = search.toString();
  return text ? `?${text}` : "";
}

function need(value, hint) {
  if (!value) throw new Error(hint);
  return value;
}

function post(path, payload) {
  return request(path, {
    method: "POST",
    headers: { "content-type": "application/json", "idempotency-key": newKey() },
    body: JSON.stringify(payload),
  });
}

async function main() {
  if (flags.project !== undefined && !project) throw new Error("--project 需要项目 ID");
  switch (command) {
    case "status":
      return request("/api/v1/system/status");
    case "content":
      return request(
        `/api/v1/content/items${query([
          ["limit", flags.limit ?? "20"],
          ["query", flags.query],
          ["sourceId", flags.source],
          ["status", flags.status],
        ])}`,
      );
    case "content-item": {
      const id = encodeURIComponent(need(rest[0], "需要 contentId"));
      const [item, body] = await Promise.all([
        request(`/api/v1/content/items/${id}`),
        request(`/api/v1/content/items/${id}/body`),
      ]);
      return { item, body };
    }
    case "sources":
      return request(`/api/v1/catalog/sources${query([["limit", flags.limit ?? "50"]])}`);
    case "targets":
      return request("/api/v1/delivery/targets");
    case "records":
      return request(
        `/api/v1/delivery/records${query([
          ["limit", flags.limit ?? "50"],
          ["type", flags.type],
          ["status", flags.status],
        ])}`,
      );
    case "deliver": {
      const contentIds = (rest[0] ?? "")
        .split(",")
        .map((item) => item.trim())
        .filter(Boolean);
      if (!contentIds.length) throw new Error("需要至少一个 contentId");
      const targets = (flags.targets ?? "")
        .split(",")
        .map((item) => item.trim())
        .filter(Boolean)
        .map((pair) => {
          const at = pair.indexOf(":");
          if (at <= 0 || at === pair.length - 1) {
            throw new Error(`目标形如 kind:id，收到: ${pair}`);
          }
          return { kind: pair.slice(0, at), id: pair.slice(at + 1) };
        });
      if (!targets.length) throw new Error("需要 --targets kind:id[,kind:id…]");
      return post("/api/v1/delivery/jobs", {
        contentIds,
        targets,
        note: flags.note,
      });
    }
    case "op":
      return request(`/api/v1/operations/${encodeURIComponent(need(rest[0], "需要 operationId"))}`);
    case "logs":
      return request(
        `/api/v1/observability/logs${query([
          ["limit", flags.limit ?? "50"],
          ["correlationId", flags.correlation],
          ["level", flags.level],
        ])}`,
      );
    default:
      process.stdout.write(usage);
      return null;
  }
}

main()
  .then((data) => {
    if (data !== null) process.stdout.write(`${JSON.stringify(data, null, 2)}\n`);
  })
  .catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
