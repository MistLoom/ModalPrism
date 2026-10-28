// 构建前契约检查：src/lib/api.ts 里的每个调用都必须命中 public-api.openapi.yaml。
// 只校验方法与路径；请求体由 Rust 侧拒绝未知字段，资源字段以响应为准。
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import YAML from "yaml";

const specPath = fileURLToPath(new URL("../../../docs/specs/public-api.openapi.yaml", import.meta.url));
const apiPath = fileURLToPath(new URL("../src/lib/api.ts", import.meta.url));
const spec = YAML.parse(fs.readFileSync(specPath, "utf8"));
const source = fs.readFileSync(apiPath, "utf8");

const operations = new Map();
for (const [path, item] of Object.entries(spec.paths ?? {})) {
  const methods = new Set(Object.keys(item).filter((key) => ["get", "post", "patch", "put", "delete"].includes(key)));
  operations.set(path, methods);
}

const calls = [];
const pattern = /"(GET|POST|PATCH|PUT|DELETE)",\s*[`"](\/[^`"]*)/g;
let match;
while ((match = pattern.exec(source))) {
  const method = match[1];
  let path = match[2].split("?")[0];
  path = path.replace(/\$\{query\([^}]*\)\}/g, "");
  path = path.replace(/\$\{[^}]+\}/g, "{id}");
  calls.push({ method, path });
}

const failures = [];
for (const call of calls) {
  let matched = null;
  for (const [path, methods] of operations) {
    const normalized = path.replace(/\{[^}]+\}/g, "{id}");
    if (normalized === call.path) {
      matched = { path, methods };
      break;
    }
    // 动作路径在 OpenAPI 里写作 /x/{id}:action，客户端按同一写法调用。
    const action = path.replace(/\{[^}]+\}/g, "{id}");
    if (action === call.path) {
      matched = { path, methods };
      break;
    }
  }
  if (!matched) {
    failures.push(`${call.method} ${call.path} 不在 OpenAPI 中`);
    continue;
  }
  if (!matched.methods.has(call.method.toLowerCase())) {
    failures.push(`${call.method} ${call.path} 的方法未在 OpenAPI 声明`);
  }
}

if (calls.length < 40) {
  failures.push(`扫描到的调用过少（${calls.length}），解析可能失效`);
}
if (failures.length) {
  console.error("公共 API 契约检查失败：");
  for (const failure of failures) console.error(`  - ${failure}`);
  process.exit(1);
}
console.log(`公共 API 契约检查通过：${calls.length} 个调用命中 OpenAPI。`);
