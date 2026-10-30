import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { fileURLToPath } from "node:url";
import test from "node:test";

const script = fileURLToPath(new URL("./prism.mjs", import.meta.url));

async function fixture(t) {
  const requests = [];
  const server = createServer(async (request, response) => {
    let body = "";
    for await (const chunk of request) body += chunk;
    requests.push({ method: request.method, url: request.url, headers: request.headers, body });
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({ project: request.headers["x-project-id"] ?? "proj_default" }));
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  t.after(() => new Promise((resolve) => server.close(resolve)));
  const base = `http://127.0.0.1:${server.address().port}`;
  const run = (args) => new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [script, ...args, "--base", base], {
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8").on("data", (chunk) => { stdout += chunk; });
    child.stderr.setEncoding("utf8").on("data", (chunk) => { stderr += chunk; });
    child.on("error", reject);
    child.on("close", (code) => resolve({ code, stdout, stderr }));
  });
  return { requests, run };
}

test("selects distinct projects for content queries", async (t) => {
  const { run, requests } = await fixture(t);
  for (const project of ["proj_alpha", "proj_beta"]) {
    const result = await run(["content", "--project", project, "--query", "agent"]);
    assert.equal(result.code, 0, result.stderr);
    assert.equal(JSON.parse(result.stdout).project, project);
  }
  assert.deepEqual(requests.map((request) => request.headers["x-project-id"]), ["proj_alpha", "proj_beta"]);
  assert.ok(requests.every((request) => request.url === "/api/v1/content/items?limit=20&query=agent"));
});

test("omits the project header when no project is supplied", async (t) => {
  const { run, requests } = await fixture(t);
  const result = await run(["sources"]);
  assert.equal(result.code, 0, result.stderr);
  assert.equal(requests[0].headers["x-project-id"], undefined);
  assert.equal(JSON.parse(result.stdout).project, "proj_default");
});

test("preserves delivery headers and payload while selecting a project", async (t) => {
  const { run, requests } = await fixture(t);
  const result = await run(["deliver", "r_first,r_second", "--targets", "model:model/example", "--project", "proj_alpha"]);
  assert.equal(result.code, 0, result.stderr);
  assert.equal(requests.length, 1);
  const request = requests[0];
  assert.equal(request.method, "POST");
  assert.equal(request.url, "/api/v1/delivery/jobs");
  assert.equal(request.headers["x-project-id"], "proj_alpha");
  assert.equal(request.headers["content-type"], "application/json");
  assert.match(request.headers["idempotency-key"], /^skill-/);
  assert.deepEqual(JSON.parse(request.body), {
    contentIds: ["r_first", "r_second"],
    targets: [{ kind: "model", id: "model/example" }],
  });
});

test("uses the selected project for both content detail requests", async (t) => {
  const { run, requests } = await fixture(t);
  const result = await run(["content-item", "r_example", "--project", "proj_beta"]);
  assert.equal(result.code, 0, result.stderr);
  assert.equal(requests.length, 2);
  assert.ok(requests.every((request) => request.headers["x-project-id"] === "proj_beta"));
  assert.deepEqual(requests.map((request) => request.url).sort(), [
    "/api/v1/content/items/r_example",
    "/api/v1/content/items/r_example/body",
  ]);
});

test("rejects a missing or blank project ID before making a request", async (t) => {
  const { run, requests } = await fixture(t);
  for (const args of [["--project"], ["--project", ""], ["--project", "   "]]) {
    const result = await run(["content", ...args]);
    assert.equal(result.code, 1);
    assert.match(result.stderr, /--project 需要项目 ID/);
  }
  assert.equal(requests.length, 0);
});

test("documents the project option in help", async (t) => {
  const { run, requests } = await fixture(t);
  const result = await run(["help"]);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /--project/);
  assert.equal(requests.length, 0);
});
