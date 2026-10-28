// RawPrism Guest Ability：用 leaf_ability_sdk 的 serveAbility 暴露扫描能力。
// 覆盖 README「支持」面：格式 TypeId、Session 扫描 / 查询 / 物化、输入方式
// （参数 / 分段 / 大文件）、ScanConfig / ParseLimits、并发（scanAsync /
// scanConcurrent / cancel）、JSONPath、有限自定义检测器。
// 不宣称：二进制输入、语义分析、网络获取、内容格式互转。
package rawprism_guest

import rawprism.*
import rawprism.core.*
import rawprism.object.*
import leaf_ability_sdk.*
import std.collection.*

// 方法注册表（与 ability.yml 的 methods 一一对应）。
func rawprismHandlers(): HashMap<String, Handler> {
    let handlers = HashMap<String, Handler>()
    handlers.add("scan", scanHandler)
    handlers.add("scan_file", scanFileHandler)
    handlers.add("scan_chunks", scanChunksHandler)
    handlers.add("materialize", materializeHandler)
    handlers.add("query_object", queryObjectHandler)
    handlers.add("custom_detect", customDetectHandler)
    handlers.add("scan_async", scanAsyncHandler)
    handlers.add("scan_concurrent", scanConcurrentHandler)
    handlers.add("scan_with_cancel", scanWithCancelHandler)
    handlers
}

main() {
    serveAbility(rawprismHandlers())
}

// ---------- 结果序列化 ----------

// 区域 → JSON（只含区域信息，不触发物化）。
func regionJson(r: Region): JsonValue {
    jObjOf([
        ("type_id", JStr(r.typeName)),
        ("start", JNum(Float64(r.start))),
        ("end", JNum(Float64(r.end))),
        ("confidence", JNum(r.confidence)),
        ("truncated", JBool(r.truncated)),
        ("errored", JBool(r.errored)),
        ("complete", JBool(r.complete)),
    ])
}

// 类型计数 → [{id, count}]。
func typeCountsJson(s: Session): JsonValue {
    let arr = ArrayList<JsonValue>()
    for ((k, v) in s.typeCounts()) {
        arr.add(jObjOf([("id", JStr(k)), ("count", JNum(Float64(v)))]))
    }
    JArr(arr)
}

// 查询结果 → {types, regions, truncated}。selectors 为空时取全部类型。
func sessionResultJson(s: Session, selectors: Array<String>): JsonValue {
    let regions = ArrayList<JsonValue>()
    let hits = if (selectors.size == 0) { s.tree.ordered() } else { hitsOf(s, selectors) }
    for (r in hits) {
        regions.add(regionJson(r))
    }
    jObjOf([
        ("types", typeCountsJson(s)),
        ("regions", JArr(regions)),
        ("truncated", JBool(s.truncated)),
    ])
}

func hitsOf(s: Session, selectors: Array<String>): ArrayList<Region> {
    let out = ArrayList<Region>()
    for (res in s.getAll(selectors)) {
        out.add(res.region)
    }
    out
}

// Obj → JSON 值（对象视图 / JSONPath 结果）。
func objToJson(o: Obj): JsonValue {
    match (o) {
        case m: ObjMap =>
            let pairs = ArrayList<(String, JsonValue)>()
            for ((k, v) in m.entries()) {
                pairs.add((k, objToJson(v)))
            }
            JObj(pairs)
        case q: ObjSeq =>
            let items = ArrayList<JsonValue>()
            for (v in q.items()) {
                items.add(objToJson(v))
            }
            JArr(items)
        case t: ObjText => JStr(t.asText() ?? "")
        case n: ObjNumber => JNum(n.asFloat() ?? 0.0)
        case b: ObjBool => JBool(b.asBool() ?? false)
        case _: ObjNull => JNull
        case _ => JStr(o.rawText())
    }
}

// ---------- 参数解析 ----------

// args.text：必填的整段文本。
func argText(args: JsonValue): String {
    match (jAsStr(jObjGet(args, "text"))) {
        case Some(t) => t
        case None => throw failSdk("缺少 text 参数", "invalid_argument")
    }
}

// args.selectors：可选的类型选择器数组。
func argSelectors(args: JsonValue): Array<String> {
    let out = ArrayList<String>()
    if (let Some(arr) <- jAsArr(jObjGet(args, "selectors"))) {
        for (v in arr) {
            if (let Some(s) <- jAsStr(v)) {
                out.add(s)
            }
        }
    }
    out.toArray()
}

// args.config → ScanConfig（含 ParseLimits 对应的深度 / 结构长度 / 截断字段）。
func argConfig(args: JsonValue): ScanConfig {
    let cfg = ScanConfig()
    let c = jObjGet(args, "config")
    if (jIsNull(c)) {
        return cfg
    }
    if (let Some(n) <- jAsNum(jObjGet(c, "max_depth"))) { cfg.maxDepth = Int64(n) }
    if (let Some(n) <- jAsNum(jObjGet(c, "max_structure_length"))) { cfg.maxStructureLength = Int64(n) }
    if (let Some(n) <- jAsNum(jObjGet(c, "max_regions"))) { cfg.maxRegions = Int64(n) }
    if (let Some(n) <- jAsNum(jObjGet(c, "max_materialize_bytes"))) { cfg.maxMaterializeBytes = Int64(n) }
    if (let Some(n) <- jAsNum(jObjGet(c, "parallelism"))) { cfg.parallelism = Int64(n) }
    if (let Some(n) <- jAsNum(jObjGet(c, "time_budget_ms"))) { cfg.timeBudgetMs = Int64(n) }
    if (let Some(n) <- jAsNum(jObjGet(c, "confidence_threshold"))) { cfg.confidenceThreshold = n }
    if (let Some(n) <- jAsNum(jObjGet(c, "max_entity_length"))) { cfg.maxEntityLength = Int64(n) }
    if (let Some(b) <- jAsBool(jObjGet(c, "truncate_on_depth"))) { cfg.truncateOnDepth = b }
    match (jAsStr(jObjGet(c, "utf8_policy"))) {
        case Some("strict") => cfg.utf8Policy = UTF8Policy.Strict
        case Some("error_region") => cfg.utf8Policy = UTF8Policy.ErrorRegion
        case Some(other) => throw failSdk("未知 utf8_policy: ${other}", "invalid_argument")
        case None => ()
    }
    cfg
}

// 与 limitsOf 对应的 ParseLimits 视图（供需要独立 ParseLimits 的调用方参考）。
func parseLimitsOf(args: JsonValue): ParseLimits {
    let cfg = argConfig(args)
    ParseLimits(maxDepth: cfg.maxDepth, maxStructureLength: cfg.maxStructureLength,
        truncateOnDepth: cfg.truncateOnDepth)
}

// ---------- 方法实现 ----------

// 新建会话并完成扫描。
func scannedSession(text: String, args: JsonValue): Session {
    let s = Session(Input(text), config: argConfig(args))
    s.scan()
    s
}

// scan：参数输入扫描。
func scanHandler(_ctx: GuestCtx, args: JsonValue): JsonValue {
    let s = scannedSession(argText(args), args)
    sessionResultJson(s, argSelectors(args))
}

// scan_file：大文件输入，按路径分段读取。
func scanFileHandler(_ctx: GuestCtx, args: JsonValue): JsonValue {
    let path = match (jAsStr(jObjGet(args, "path"))) {
        case Some(p) => p
        case None => throw failSdk("缺少 path 参数", "invalid_argument")
    }
    let s = Session(Input.fromFile(path), config: argConfig(args))
    while (s.input.hasMore()) {}
    s.scan()
    sessionResultJson(s, argSelectors(args))
}

// scan_chunks：分段输入，按序拼接扫描。
func scanChunksHandler(_ctx: GuestCtx, args: JsonValue): JsonValue {
    let chunks = ArrayList<String>()
    match (jAsArr(jObjGet(args, "chunks"))) {
        case Some(arr) =>
            for (v in arr) {
                match (jAsStr(v)) {
                    case Some(t) => chunks.add(t)
                    case None => throw failSdk("chunks 元素须为字符串", "invalid_argument")
                }
            }
        case None => throw failSdk("缺少 chunks 参数", "invalid_argument")
    }
    if (chunks.size == 0) {
        throw failSdk("chunks 不能为空", "invalid_argument")
    }
    let retention = match (jAsNum(jObjGet(args, "retention_limit"))) {
        case Some(n) => Int64(n)
        case None => -1
    }
    let src = StringChunkSource(chunks.toArray())
    let s = Session(Input(src, retentionLimit: retention), config: argConfig(args))
    while (s.input.hasMore()) {}
    s.scan()
    sessionResultJson(s, argSelectors(args))
}

// materialize：按类型选择器取第 index 个区域的原文与（可选）对象值。
func materializeHandler(_ctx: GuestCtx, args: JsonValue): JsonValue {
    let s = scannedSession(argText(args), args)
    let selectors = argSelectors(args)
    if (selectors.size == 0) {
        throw failSdk("缺少 selectors 参数", "invalid_argument")
    }
    let index = match (jAsNum(jObjGet(args, "index"))) {
        case Some(n) => Int64(n)
        case None => 0
    }
    let hits = hitsOf(s, selectors)
    if (index < 0 || index >= hits.size) {
        throw failSdk("index 越界: ${index} / ${hits.size}", "invalid_argument")
    }
    let r = hits[index]
    let wantValue = jAsBool(jObjGet(args, "with_value")) ?? false
    let pairs = ArrayList<(String, JsonValue)>()
    match (regionJson(r)) {
        case JObj(ps) =>
            for (p in ps) {
                pairs.add(p)
            }
        case _ => ()
    }
    // 原文按区间从输入直接取（对象视图的 rawText 依赖 inputRef，可能为空）。
    pairs.add(("text", JStr(s.input.text(r.span))))
    if (wantValue) {
        pairs.add(("value", objToJson(s.objectOf(r))))
    }
    JObj(pairs)
}

// query_object：统一 Object 视图 + JSONPath 查询。
func queryObjectHandler(_ctx: GuestCtx, args: JsonValue): JsonValue {
    let s = scannedSession(argText(args), args)
    let selectors = argSelectors(args)
    if (selectors.size == 0) {
        throw failSdk("缺少 selectors 参数", "invalid_argument")
    }
    let hits = hitsOf(s, selectors)
    let objects = ArrayList<JsonValue>()
    for (r in hits) {
        objects.add(objToJson(s.objectOf(r)))
    }
    let pairs = ArrayList<(String, JsonValue)>()
    pairs.add(("objects", JArr(objects)))
    // 可选 JSONPath：query 作用于第一个区域的 Object（无区域时报 not_found）。
    match (jAsStr(jObjGet(args, "query"))) {
        case Some(q) =>
            if (hits.size == 0) {
                throw failSdk("无可查询对象", "not_found")
            }
            let root = s.objectOf(hits[0])
            let found = ArrayList<JsonValue>()
            for (hit in jsonPath(root, q)) {
                found.add(objToJson(hit))
            }
            pairs.add(("query", JStr(q)))
            pairs.add(("hits", JArr(found)))
        case None => ()
    }
    JObj(pairs)
}

// custom_detect：有限自定义检测器（起止标记）。
func customDetectHandler(_ctx: GuestCtx, args: JsonValue): JsonValue {
    let typeIdText = match (jAsStr(jObjGet(args, "type_id"))) {
        case Some(t) => t
        case None => throw failSdk("缺少 type_id 参数", "invalid_argument")
    }
    let startMarker = match (jAsStr(jObjGet(args, "start_marker"))) {
        case Some(m) => m
        case None => throw failSdk("缺少 start_marker 参数", "invalid_argument")
    }
    if (startMarker == "") {
        throw failSdk("start_marker 不能为空", "invalid_argument")
    }
    let endMarker = jAsStr(jObjGet(args, "end_marker")) ?? ""
    let registry = GetterRegistry()
    let _ = registry.register(MarkerDetector(typeIdText, startMarker, endMarker))
    let s = Session(Input(argText(args)), config: argConfig(args), registry: registry)
    s.scan()
    sessionResultJson(s, [typeIdText])
}

// scan_async：异步入口 scanAsync。
func scanAsyncHandler(_ctx: GuestCtx, args: JsonValue): JsonValue {
    let fut = scanAsync(Input(argText(args)), config: argConfig(args))
    let s = fut.get()
    sessionResultJson(s, argSelectors(args))
}

// scan_concurrent：并行扫描 scanConcurrent。
func scanConcurrentHandler(_ctx: GuestCtx, args: JsonValue): JsonValue {
    let s = Session(Input(argText(args)), config: argConfig(args))
    scanConcurrent(s)
    sessionResultJson(s, argSelectors(args))
}

// scan_with_cancel：启动异步扫描后立即请求取消，返回已产生的部分结果。
func scanWithCancelHandler(_ctx: GuestCtx, args: JsonValue): JsonValue {
    let fut = scanAsync(Input(argText(args)), config: argConfig(args))
    let s = fut.get()
    cancelScan(s)
    sessionResultJson(s, argSelectors(args))
}

// ---------- 有限自定义检测器 ----------

// 起止标记检测器：在 scope 内按字节查找 startMarker，遇到 endMarker（缺省为行尾）闭合。
public class MarkerDetector <: Detector {
    let typeIdText: String
    let startMarker: String
    let endMarker: String

    public init(typeIdText: String, startMarker: String, endMarker: String) {
        this.typeIdText = typeIdText
        this.startMarker = startMarker
        this.endMarker = endMarker
    }

    public prop typeId: TypeId {
        get() { TypeId(typeIdText) }
    }
    public prop within: Array<String> {
        get() { [] }
    }
    public prop dependsOn: Array<TypeId> {
        get() { [] }
    }
    public prop authoritative: Array<TypeId> {
        get() { [] }
    }

    public func detect(scope: Span, ctx: DetectContext): Unit {
        let data = ctx.session.inputBytes()
        let start = startMarker.toArray()
        let end = endMarker.toArray()
        var i = scope.start
        while (i + start.size <= scope.end) {
            if (!bytesMatch(data, i, start, scope.end)) {
                i += 1
                continue
            }
            var e = i + start.size
            var closed = false
            while (e < scope.end) {
                if (end.size > 0 && bytesMatch(data, e, end, scope.end)) {
                    e += end.size
                    closed = true
                    break
                }
                if (end.size == 0 && data[e] == 10) { // 缺省以行尾闭合
                    closed = true
                    break
                }
                e += 1
            }
            if (!closed) {
                e = scope.end
            }
            let _ = ctx.emit(Span(i, e), typeId, 1.0, HashMap<String, String>())
            i = e
        }
    }

    func bytesMatch(data: Array<UInt8>, at: Int64, marker: Array<UInt8>, limit: Int64): Bool {
        if (at + marker.size > limit) {
            return false
        }
        for (k in 0..marker.size) {
            if (data[at + k] != marker[k]) {
                return false
            }
        }
        true
    }
}
