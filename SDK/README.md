# Ability Guest SDK — 参考布局

各语言 Guest SDK（`ability-sdk` / `-ts` / `-py` / `-cj`）都讲同一套 `guest-protocol-1` stdio JSON 行帧。
**新增第五种语言时，请按下列分层对照移植**，命名与职责尽量对齐，避免再出现「某一语有 flush / 长任务、另一语没有」的静默分叉。

## 分层（谁管什么）

| 层 | 职责 | Rust | Node | Python | 仓颉 |
|----|------|------|------|--------|------|
| **protocol** | 帧编解码、`frame_reply` / 错误体 | `protocol`（crate 内） | `lib/protocol.js` | `protocol.py` | `protocol.cj` |
| **transport / bridge** | stdio 读写、**写后 flush**、stdin EOF→退出、`send` / `request` / `pump_line` | `guest.rs` `serve` / `StdoutSink` | `guest.js` `createBridge` + `serveStdio` | `guest.py` `_SyncBridge` + `serve_stdio` | `guest.cj` `Bridge` + `serveWithIO`；`ability.cj` `stdioWrite`/`stdioInput` |
| **ctx** | `GuestCtx` 薄转发 Host `CtxOp` | `guest.rs` ctx | `guest.js` ctx 对象 | `guest.py` ctx | `guest.cj` `GuestCtx` |
| **spec / ability** | `AbilitySpec`、`ability.yml` 装载、`serveAbility` | （作者侧 in-process） | `ability.js` | `ability.py` | `ability.cj` + `spec.cj` |
| **长任务** | `pending` + `{handle}` 轮询（协议级） | 不强制 | `pending` / `withLongTask` | `pending` / `with_long_task` | `pending` / `withLongTask` |

## 推荐主循环形状

1. `resolve_ability_root`（显式 root → `LEAF_ABILITY_ROOT` → 上溯找 `ability.yml`）  
2. `load_ability_yaml` → `build_provider`（methods 用长任务包装）  
3. `serve_*(provider)`：读行 → 忽略空行 → `dispatch` 应答 / 侧路事件 → 处理 `hello`/`create`/`invoke`/`lifecycle`/`shutdown`  
4. **每帧** `write(line)` 后 **flush**；`readline` 得 EOF / 异常 → **干净 return**

## 对照清单

仓库根 README 的「Guest SDK 对等清单」是活表；改协议能力时同步更新。契约 MUST 见 `manual/ability-guest-protocol.zh-CN.md`。

## 禁止

- 只在产品仓改某一语言 SDK 热修、不上提本目录。  
- 某一语宣称支持长任务 / 某 CtxOp，但矩阵未标 ✅。  
