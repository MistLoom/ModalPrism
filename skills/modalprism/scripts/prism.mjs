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

  health                              服务状态与配置代际
  articles [--limit N] [--source S] [--q 关键词]
  article <rid>
  sources
  workflows
  runs [--limit N]
  run <slug> <rid>
  process
`;

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
    if (value !== undefined) {
      search.set(key, value);
    }
  }
  const text = search.toString();
  return text ? `?${text}` : "";
}

function need(value, hint) {
  if (!value) {
    throw new Error(hint);
  }
  return value;
}

async function main() {
  switch (command) {
    case "health":
      return request("/api/v1/health");
    case "articles":
      return request(`/api/v1/articles${query([["limit", flags.limit], ["source", flags.source], ["q", flags.q]])}`);
    case "article":
      return request(`/api/v1/articles/${encodeURIComponent(need(rest[0], "需要 rid"))}`);
    case "sources":
      return request("/api/v1/sources");
    case "workflows":
      return request("/api/v1/workflows");
    case "runs":
      return request(`/api/v1/runs${query([["limit", flags.limit]])}`);
    case "run":
      return request(`/api/v1/workflows/${encodeURIComponent(need(rest[0], "需要工作流 slug"))}/run`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ rid: need(rest[1], "需要 rid") }),
      });
    case "process":
      return request("/api/v1/process", { method: "POST" });
    default:
      process.stdout.write(usage);
      return null;
  }
}

main()
  .then((data) => {
    if (data !== null) {
      process.stdout.write(`${JSON.stringify(data, null, 2)}\n`);
    }
  })
  .catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
