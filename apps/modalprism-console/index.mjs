#!/usr/bin/env node
// ModalPrism 控制台 Node 进程。
//
// 只做四件事：托管 SPA、SPA fallback、健康页、把 /api/v1 原样代理给 collector。
// 不解析业务响应、不聚合接口、不缓存、不保存 Operation。

import http from "node:http";
import net from "node:net";
import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { extname, join, normalize, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const args = process.argv.slice(2);
const options = {
  bind: "127.0.0.1:17890",
  backend: "http://127.0.0.1:17880",
  web: resolve(here, "dist"),
};
for (let index = 0; index < args.length; index += 1) {
  const flag = args[index];
  const value = args[index + 1];
  if (!value) {
    fail(`缺少参数值: ${flag}`);
  }
  index += 1;
  if (flag === "--bind" || flag === "--http") options.bind = value;
  else if (flag === "--backend" || flag === "--ops") options.backend = value;
  else if (flag === "--web") options.web = resolve(value);
  else fail(`未知参数: ${flag}`);
}

const [host, portText] = splitHostPort(options.bind);
if (!isLoopbackHost(host)) {
  fail(`监听地址必须是回环地址: ${host}`);
}
const port = Number(portText);
if (!Number.isInteger(port) || port <= 0 || port > 65535) {
  fail(`监听端口无效: ${portText}`);
}
const backend = new URL(options.backend);
if (!["127.0.0.1", "localhost", "::1", "[::1]"].includes(backend.hostname)) {
  fail(`collector 地址必须是回环地址: ${backend.hostname}`);
}

const MIME = new Map([
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".mjs", "text/javascript; charset=utf-8"],
  [".css", "text/css; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
  [".svg", "image/svg+xml"],
  [".png", "image/png"],
  [".jpg", "image/jpeg"],
  [".jpeg", "image/jpeg"],
  [".webp", "image/webp"],
  [".ico", "image/x-icon"],
  [".woff2", "font/woff2"],
  [".map", "application/json; charset=utf-8"],
]);

const server = http.createServer((request, response) => {
  handle(request, response).catch((error) => {
    console.error("modalprism-console: 请求处理失败", error);
    if (!response.headersSent) {
      response.writeHead(500, { "content-type": "application/json; charset=utf-8" });
      response.end(
        JSON.stringify({
          error: {
            code: "internal",
            message: "服务内部错误",
            fieldErrors: [],
            requestId: `node_${Date.now().toString(36)}`,
            retryable: false,
          },
        }),
      );
    } else {
      response.destroy();
    }
  });
});

async function handle(request, response) {
  // 只接受 origin-form；absolute-form 或 //host 会把代理目标劫持到请求方指定的主机。
  if (typeof request.url !== "string" || !request.url.startsWith("/") || request.url.startsWith("//")) {
    response.writeHead(400, { "content-type": "application/json; charset=utf-8" });
    response.end(
      JSON.stringify({
        error: {
          code: "invalidRequest",
          message: "请求目标必须是 / 开头的路径",
          fieldErrors: [],
          requestId: `node_${Date.now().toString(36)}`,
          retryable: false,
        },
      }),
    );
    return;
  }
  const url = new URL(request.url, "http://console.invalid");
  if (url.pathname === "/healthz") {
    return health(response);
  }
  if (url.pathname === "/api/v1" || url.pathname.startsWith("/api/v1/")) {
    return proxy(request, response, url.pathname + url.search);
  }
  return serveStatic(request, response, url.pathname);
}

async function health(response) {
  const started = Date.now();
  let reachable = false;
  let status = null;
  try {
    const reply = await fetch(new URL("/api/v1/system/status", backend), {
      signal: AbortSignal.timeout(2000),
    });
    reachable = reply.ok;
    if (reply.ok) status = (await reply.json()).status ?? null;
  } catch {
    reachable = false;
  }
  const body = JSON.stringify({
    status: "ok",
    backend: { reachable, status },
    latencyMs: Date.now() - started,
    startedAt: STARTED_AT,
  });
  response.writeHead(200, {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store",
  });
  response.end(body);
}

function proxy(request, response, path) {
  const headers = { ...request.headers };
  delete headers.host;
  delete headers.connection;
  delete headers["transfer-encoding"];
  const upstream = http.request(
    {
      // 上游固定为启动参数中的 collector，路径只取请求目标，不接受绝对 URI。
      hostname: backend.hostname.replace(/^\[|\]$/g, ""),
      port: backend.port || 80,
      path,
      method: request.method,
      headers,
    },
    (reply) => {
      const forwarded = { ...reply.headers };
      delete forwarded.connection;
      delete forwarded["transfer-encoding"];
      response.writeHead(reply.statusCode ?? 502, forwarded);
      reply.pipe(response);
    },
  );
  upstream.on("error", (error) => {
    if (response.headersSent) {
      response.destroy();
      return;
    }
    const requestId = `node_${Date.now().toString(36)}`;
    response.writeHead(502, {
      "content-type": "application/json; charset=utf-8",
      "x-request-id": requestId,
      "cache-control": "no-store",
    });
    response.end(
      JSON.stringify({
        error: {
          code: "dependencyUnavailable",
          message: `collector 不可达: ${error.message}`,
          fieldErrors: [],
          requestId,
          retryable: true,
        },
      }),
    );
  });
  request.pipe(upstream);
}

async function serveStatic(request, response, pathname) {
  if (request.method !== "GET" && request.method !== "HEAD") {
    response.writeHead(405, { allow: "GET, HEAD" });
    response.end();
    return;
  }
  let relative;
  try {
    relative = decodeURIComponent(pathname).replace(/^\/+/, "");
  } catch {
    response.writeHead(400, { "content-type": "text/plain; charset=utf-8" });
    response.end("bad request");
    return;
  }
  const candidate = normalize(join(options.web, relative));
  if (candidate !== options.web && !candidate.startsWith(`${options.web}${sep}`)) {
    response.writeHead(403);
    response.end();
    return;
  }
  const file = await readable(candidate);
  if (file) {
    return send(response, file, cacheFor(file), request.method === "HEAD");
  }
  const index = join(options.web, "index.html");
  const fallback = await readable(index);
  if (!fallback) {
    response.writeHead(503, { "content-type": "text/plain; charset=utf-8" });
    response.end("前端产物尚未构建：在 apps/modalprism-console 执行 pnpm install && pnpm build");
    return;
  }
  return send(response, fallback, "no-cache", request.method === "HEAD");
}

async function readable(path) {
  try {
    const info = await stat(path);
    return info.isFile() ? { path, info } : null;
  } catch {
    return null;
  }
}

function cacheFor(file) {
  return file.path.includes(`${join(options.web, "assets")}`) ? "public, max-age=31536000, immutable" : "no-cache";
}

function send(response, file, cache, headOnly) {
  response.writeHead(200, {
    "content-type": MIME.get(extname(file.path).toLowerCase()) ?? "application/octet-stream",
    "content-length": file.info.size,
    "cache-control": cache,
  });
  if (headOnly) {
    response.end();
    return;
  }
  createReadStream(file.path).pipe(response);
}

function splitHostPort(value) {
  if (value.startsWith("[")) {
    const end = value.indexOf("]");
    return [value.slice(1, end), value.slice(end + 2)];
  }
  const at = value.lastIndexOf(":");
  return [value.slice(0, at), value.slice(at + 1)];
}

function isLoopbackHost(host) {
  if (host === "localhost" || host === "::1" || host === "[::1]") return true;
  const address = net.isIP(host);
  return address === 4 && host.startsWith("127.");
}

function fail(message) {
  console.error(`modalprism-console: ${message}`);
  process.exit(1);
}

const STARTED_AT = new Date().toISOString();
server.listen(port, host, () => {
  console.log(
    `modalprism-console: node=http://${host}:${port} backend=${backend.origin} web=${options.web}`,
  );
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    server.close(() => process.exit(0));
  });
}
