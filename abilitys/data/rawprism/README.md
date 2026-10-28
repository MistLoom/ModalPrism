# rawprism（仓颉 Guest Ability）

> **ModalPrism 落位说明（产品 ability，非 leaf_builtin）**
>
> - 路径：bilitys/data/rawprism（跨应用共享 data 类 ability）
> - 来源：自 D:\Project\Leaf\external\rawprism 复制（不含 	arget/）
> - cjpm.toml 依赖已改为相对本仓：SDK/ability-sdk-cj；RawPrism 库指向仓库外同级 D:\Project\RawPrism（../../../../RawPrism）
> - **Host 缺口**：当前 LEAF Host/bility-support 的 RuntimeKind 仅有 Rust / Node / Python；ramework_version: cangjie@1 **尚不能被 Host 加载或 compose 编入**。落位仅供后续仓颉运行时/桥接接线，**不宣称已可运行**。
> - 尚未写入 Cargo.toml workspace，亦未在 collector 做 install 接线。

# rawprism（仓颉 Guest Ability）

对接 [RawPrism](../../../RawPrism)（纯仓颉文本扫描分类库）的 Leaf Guest Ability。
输入文本，识别结构化内容、非结构化内容与实体，返回相对整段输入的偏移区间与类型，
并支持惰性物化、统一 Object 视图、JSONPath 查询与并发扫描。

- `framework_version`: `cangjie@1`（guest-protocol-1）
- 入口：`src/serve.cj`，通过 `SDK/ability-sdk-cj` 的 `serveAbility` 注册方法
- 运行时依赖：`rawprism`（path）、`leaf_ability_sdk`（path）

## 方法

| 方法 | 说明 | 关键参数 |
|------|------|----------|
| `scan` | 参数输入扫描，返回 `{types, regions, truncated}` | `text`、`config?`、`selectors?` |
| `scan_file` | 大文件输入：按路径分段读取扫描 | `path`、`config?`、`selectors?` |
| `scan_chunks` | 分段输入：按序拼接扫描 | `chunks[]`、`retention_limit?`、`config?`、`selectors?` |
| `materialize` | 惰性物化：取区域原文 / 对象值 | `text`、`selectors`、`index?=0`、`with_value?=false` |
| `query_object` | 统一 Object 视图 + JSONPath（RFC 9535）查询 | `text`、`selectors`、`query?` |
| `custom_detect` | 有限自定义检测器：按起止标记声明自定义 TypeId | `text`、`type_id`、`start_marker`、`end_marker?`（缺省行尾闭合） |
| `scan_async` | 异步入口（RawPrism `scanAsync`） | 同 `scan` |
| `scan_concurrent` | 并行扫描（`scanConcurrent`，并行度由 `config.parallelism` 约束） | 同 `scan` |
| `scan_with_cancel` | 启动异步扫描后立即请求取消（`cancelScan`），返回部分结果 | 同 `scan` |

`config`（可选，映射 RawPrism `ScanConfig` / `ParseLimits`）：
`max_depth`、`max_structure_length`、`max_regions`、`max_materialize_bytes`、
`parallelism`、`time_budget_ms`、`utf8_policy`（`strict` / `error_region`）、
`truncate_on_depth`、`confidence_threshold`、`max_entity_length`。
超限返回带 `truncated` 标记的部分结果。

`regions[]` 元素：`type_id`、`start`、`end`（相对整段输入的 UTF-8 字节偏移，`[start, end)`）、
`confidence`、`truncated`、`errored`、`complete`。

## 能力覆盖与边界

**支持**：格式 TypeId（`struct.json` / `struct.ndjson` / `struct.yaml` / `struct.toml` /
`struct.csv` / `struct.xml` / `markup.html` / `text.md` / `entity.url` / `entity.email`）、
Session 扫描 / 查询（`getAll` / `first` / `typeCounts` / `walk` / `chainAt` /
`intersecting`）/ 惰性物化、三种输入方式（参数 / 分段 / 大文件）及其一致性、
`ScanConfig` / `ParseLimits` 资源上限、并发（`scanAsync` / `scanConcurrent` / `cancelScan`）、
JSONPath 查询、有限自定义检测器（起止标记形式）。

**不支持**（RawPrism 未提供，本 Ability 不宣称）：二进制输入（图片 / 音视频 / 压缩包 /
PDF / Office）、语义分析、网络获取、内容格式互转。
`@Detector` 属性宏等深度定制请直接依赖 RawPrism 库使用。

## 示例

见 [`examples/`](./examples/)。以下为通过宿主对 Guest 实例 `invoke` 的参数形态：

```json
{"method": "scan", "args": {"text": "# note\n{\"a\": 1}\nhttps://example.com", "selectors": ["struct.json"]}}
```

返回：

```json
{
  "types": [{"id": "struct.json", "count": 1}, {"id": "text.md", "count": 3}, {"id": "entity.url", "count": 1}],
  "regions": [{"type_id": "struct.json", "start": 7, "end": 17, "confidence": 1, "truncated": false, "errored": false, "complete": true}],
  "truncated": false
}
```

JSONPath 查询：

```json
{"method": "query_object", "args": {"text": "{\"a\": {\"b\": [10, 20, 30]}}", "selectors": ["struct.json"], "query": "$.a.b[*]"}}
```

自定义检测器（提取 `[[o1]]` 形态的工单）：

```json
{"method": "custom_detect", "args": {"text": "a [[o1]] b", "type_id": "custom.order", "start_marker": "[[", "end_marker": "]]"}}
```

## 构建与测试

```bash
cd external/rawprism
cjpm build   # 构建 Guest 入口
cjpm test    # 内部完整测试（测例不入公开树）
```

内部测试矩阵对齐 RawPrism 各 `*_test.cj` 能力域（核心类型、格式解析、会话查询与物化、
输入方式、资源上限、并发、JSONPath、自定义检测器与宏、覆盖 / 预扫描、线协议端到端）。
