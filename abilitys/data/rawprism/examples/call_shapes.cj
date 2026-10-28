// RawPrism Guest Ability 调用示例（参数形态参考，无 @Test）。
//
// 本 Ability 由宿主以进程方式拉起（cjpm build 产物），宿主通过 guest-protocol-1
// 帧对实例 invoke。以下展示各方法的最小调用参数与期望返回结构。
package rawprism_guest

import leaf_ability_sdk.*
import std.collection.*

// 1. 扫描混合文本（参数输入）：
//    invoke("scan", jObjOf([
//        ("text", JStr("# note\n{\"a\": 1}\nhttps://example.com")),
//        ("selectors", jArrOf([JStr("struct.json"), JStr("entity.url")])),
//    ]))
//    返回：{"types": [...], "regions": [{"type_id", "start", "end", ...}], "truncated": false}

// 2. 分段输入（边界可落在 UTF-8 多字节字符内部）：
//    invoke("scan_chunks", jObjOf([
//        ("chunks", jArrOf([JStr("{\"na"), JStr("me\": 1}")])),
//        ("retention_limit", JNum(4096.0)),
//    ]))

// 3. 大文件输入（按路径分段读取，不发起网络请求）：
//    invoke("scan_file", jObjOf([("path", JStr("path/to/log.txt"))]))

// 4. 惰性物化（原文与对象值）：
//    invoke("materialize", jObjOf([
//        ("text", JStr("{\"name\": \"rp\", \"n\": 3}")),
//        ("selectors", jArrOf([JStr("struct.json")])),
//        ("with_value", JBool(true)),
//    ]))
//    返回：{"type_id": "struct.json", ..., "text": "...", "value": {"name": "rp", "n": 3}}

// 5. JSONPath 查询（RFC 9535）：
//    invoke("query_object", jObjOf([
//        ("text", JStr("{\"a\": {\"b\": [10, 20, 30]}}")),
//        ("selectors", jArrOf([JStr("struct.json")])),
//        ("query", JStr("$.a.b[*]")),
//    ]))
//    返回：{"objects": [...], "query": "$.a.b[*]", "hits": [10, 20, 30]}

// 6. 有限自定义检测器（起止标记）：
//    invoke("custom_detect", jObjOf([
//        ("text", JStr("a [[o1]] b [[o2]] c")),
//        ("type_id", JStr("custom.order")),
//        ("start_marker", JStr("[[")),
//        ("end_marker", JStr("]]")),
//    ]))

// 7. 并发扫描与取消：
//    invoke("scan_async", jObjOf([("text", JStr("{\"a\": 1} https://u.io"))]))
//    invoke("scan_concurrent", jObjOf([
//        ("text", JStr("{\"a\": 1} mid https://u.io tail")),
//        ("config", jObjOf([("parallelism", JNum(4.0))])),
//    ]))
//    invoke("scan_with_cancel", jObjOf([("text", JStr("https://u.io {\"a\": 1}"))]))

// 资源上限示例（config 映射 ScanConfig / ParseLimits）：
//    invoke("scan", jObjOf([
//        ("text", JStr(text)),
//        ("config", jObjOf([
//            ("max_regions", JNum(1000.0)),
//            ("max_depth", JNum(64.0)),
//            ("time_budget_ms", JNum(500.0)),
//            ("utf8_policy", JStr("error_region")),
//        ])),
//    ]))

main() {
    // 示例仅为参数形态说明；实际运行由 Leaf 宿主经 serveAbility 驱动。
    print("rawprism guest examples: see comments in examples/call_shapes.cj\n")
}
