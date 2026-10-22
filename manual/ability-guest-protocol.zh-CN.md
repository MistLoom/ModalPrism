# Guest Ability 线协议

面向在 Node / Python 中编写并接入 Leaf 的能力包。帧格式与版本字段的机器可读摘要随框架发布（`guest-protocol-1` 规范）。

## 版本

| 名称 | 含义 |
|------|------|
| `protocol_version` | 帧与 `CtxOp` 形状，当前 `1` |
| `framework_version` | `ability.yml` / `AbilityManifest` 语义，与框架声明版本对齐 |

二者独立演进。

## 传输

UTF-8 JSON，一行一帧，以 `\n` 结束。空行忽略。

## 握手

1. 宿主 → `{"id":"…","body":{"type":"hello"}}`
2. 客端 → `reply_to` 指向请求 id，`body.type=manifest`
3. 宿主 → `create` + `config`
4. 客端 → `ok`

之后可交错 `invoke`、`lifecycle`、`ctx`、`event`、`run_handle`；结束发 `shutdown`。

## 能力矩阵

| 能力 | 进程内 Rust | Guest（Rust / Node / Python） |
|------|-------------|-------------------------------|
| manifest / methods / lifecycle | 是 | 是 |
| `ctx.call` / `dep` / `catalog` | 是 | 是（`Ctx` 帧） |
| `local_get` / `local_set` | 是 | 是 |
| `emit` / `subscribe` / `handle` / `event_next` | 是 | 是 |
| `load` / `unload` | 是 | 是 |
| `feedback` | 是 | 是 |
| `chain_status` / `share` | 是 | 是 |
| `push_effect` + invoke 边界 undo | 是 | 是 |
| `register_aspect` | 是 | **否** |

## 能力包形态（与 Rust 共用 `ability.yml`）

| 运行时 | `framework_version` | 工程特征 | Guest 入口（固定） |
|--------|-------------------|----------|-------------------|
| Rust | `1` 或 `rust@1` | `Cargo.toml` | — |
| Node | `node@1`（`ts@1` 同义） | `package.json` | `src/serve.mjs` |
| Python | `python@1`（`py@1` 同义） | `pyproject.toml` | `src/serve.py` |

未写前缀时按目录推断：有 `Cargo.toml` 为 Rust；否则存在 `src/serve.mjs` / `src/serve.py`（或仅 `package.json` / `pyproject.toml`）。冲突时须显写前缀。

入口文件通过 Leaf Guest SDK 的 `serveAbility`（Node）或 `serve_ability`（Python），在固定路径注册 `ability.yml` 中的方法 handler。仓内 SDK 位于仓库根 `SDK/ability-sdk-ts`、`SDK/ability-sdk-py`（Rust 线协议库为 `SDK/ability-sdk`）。
