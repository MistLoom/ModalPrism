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
    flags[key] = "true";
  } else {
    flags[key] = next;
    index += 1;
  }
}

const base = (flags.base ?? process.env.MODALPRISM_BASE ?? DEFAULT_BASE).replace(/\/+$/, "");

const usage = `用法: node prism.mjs <命令> [参数] [--base http://host:port]

  status                                    系统状态与配置代际
  content [--query Q] [--source ID] [--status active|tombstoned] [--limit N]
  content-item <contentId>                  内容详情与正文
  sources [--limit N]
  workflows                                 工作流身份列表
  versions <workflowId>                     工作流版本
  inputs [--limit N]                        输入集
  input-text <name> <text>                  冻结一个文本输入集
  runs [--limit N] [--status S]             生产运行
  run <inputSetId> <workflowVersionId>      创建生产运行（返回 Operation）
  candidates [--status S]                   候选
  review <candidateId> approve|reject|returnForRegeneration [--comment C]
  op <operationId>                          Operation 状态
  logs [--correlation ID] [--level L] [--limit N]
`;

function newKey() {
  return `skill-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

async function request(path, init) {
  const response = await fetch(`${base}${path}`, init);
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
    case "workflows":
      return request("/api/v1/production/workflows?limit=100");
    case "versions":
      return request(`/api/v1/production/workflows/${encodeURIComponent(need(rest[0], "需要 workflowId"))}/versions?limit=100`);
    case "inputs":
      return request(`/api/v1/production/inputSets${query([["limit", flags.limit ?? "50"]])}`);
    case "input-text":
      return post("/api/v1/production/inputSets", {
        name: need(rest[0], "需要输入集名称"),
        inputs: [{ kind: "text", text: need(rest.slice(1).join(" "), "需要文本") }],
      });
    case "runs":
      return request(`/api/v1/production/runs${query([["limit", flags.limit ?? "50"], ["status", flags.status]])}`);
    case "run":
      return post("/api/v1/production/runs", {
        inputSetId: need(rest[0], "需要 inputSetId"),
        workflowVersionId: need(rest[1], "需要 workflowVersionId"),
        mode: "manual",
      });
    case "candidates":
      return request(`/api/v1/production/candidates${query([["limit", flags.limit ?? "50"], ["status", flags.status]])}`);
    case "review": {
      const candidateId = need(rest[0], "需要 candidateId");
      const decision = need(rest[1], "需要 decision");
      const page = await request("/api/v1/production/candidates?limit=200");
      const candidate = (page.items ?? []).find((item) => item.id === candidateId);
      if (!candidate) throw new Error(`候选不存在: ${candidateId}`);
      return request(`/api/v1/production/candidates/${encodeURIComponent(candidateId)}/reviews`, {
        method: "POST",
        headers: { "content-type": "application/json", "idempotency-key": newKey(), "if-match": candidate.etag },
        body: JSON.stringify({ decision, comment: flags.comment ?? "" }),
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
