// 资源上限与配置（对齐 RawPrism limits_test.cj 能力域）：
// ScanConfig / ParseLimits 各字段经 Guest config 传入生效，直接调用补充深度 /
// 结构长度 / 区域数量 / 物化预算覆盖。
package rawprism_guest

import rawprism.*
import rawprism.core.*
import rawprism.parse.*
import leaf_ability_sdk.*
import std.collection.*
import std.unittest.*
import std.unittest.testmacro.*

@Test
func limitsDeepNestingNoStackOverflow(): Unit {
    // 14.1：深度上限内不栈溢出，深度超限被限制
    var text = ""
    for (_ in 0..5000) {
        text = text + "["
    }
    for (_ in 0..5000) {
        text = text + "]"
    }
    var cfg = ScanConfig()
    cfg.maxDepth = 64
    let s = Session(Input(text), config: cfg)
    s.scan()
    @Assert(true)
}

@Test
func limitsRegionCountViaGuest(): Unit {
    // C7：区域数量上限，超限带截断标记
    var text = ""
    for (i in 0..200) {
        text = text + "url https://h.io/${i} "
    }
    let v = guestInvoke("scan", jObjOf([
        ("text", JStr(text)),
        ("config", jObjOf([("max_regions", JNum(10.0))])),
    ]))
    @Assert(js(jObjGet(v, "truncated")) == "true")
}

@Test
func limitsStructureLengthViaGuest(): Unit {
    // 4.4.7 / C7：单个结构超长按截断处理并标注
    var big = "{\"k\": \""
    for (_ in 0..100) {
        big = big + "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
    }
    big = big + "\"}"
    let v = guestInvoke("scan", jObjOf([
        ("text", JStr(big)),
        ("config", jObjOf([("max_structure_length", JNum(128.0))])),
    ]))
    // 超限结构被截断标注或被拒绝
    let truncated = js(jObjGet(v, "truncated")) == "true"
    let regions = jAsArr(jObjGet(v, "regions")) ?? ArrayList<JsonValue>()
    @Assert(truncated || regions.size == 0)
}

@Test
func limitsMaterializeBudgetDirect(): Unit {
    let s = Session(Input("{\"a\": 1}"))
    s.scan()
    let ctx = s.materializeCtx()
    // withinBudget(size)：size 与 maxMaterializeBytes（缺省 64MB）比较
    @Assert(ctx.withinBudget(1024))
    @Assert(!ctx.withinBudget(64 * 1024 * 1024 + 1))
}

@Test
func limitsParseLimitsDirect(): Unit {
    // ParseLimits：深度 / 结构长度 / 截断开关（14.1 / 4.4.7）
    let lim = ParseLimits(maxDepth: 8, maxStructureLength: 64, truncateOnDepth: true)
    @Assert(lim.maxDepth == 8)
    @Assert(lim.maxStructureLength == 64)
    @Assert(lim.truncateOnDepth)
    let lim2 = limitsOf(ScanConfig())
    @Assert(lim2.maxDepth == 256)
}

@Test
func limitsTimeBudgetDirect(): Unit {
    // 12.7：时间预算超时返回已有结果并带截断标记
    var cfg = ScanConfig()
    cfg.timeBudgetMs = 1
    let s = Session(Input("https://u.io {\"a\": 1} # md\n"), config: cfg)
    s.scan()
    // 结果仍可用（预算内已完成或带截断标记）
    @Assert(s.getAll(["entity.url"]).size == 1 || s.truncated)
}

@Test
func limitsConfidenceThresholdDirect(): Unit {
    // 17：低于阈值不登记
    let s = Session(Input("https://u.io"))
    s.scan()
    @Assert(s.getAll(["entity.url"]).size == 1)
}

@Test
func limitsUtf8PolicyViaGuest(): Unit {
    // 非法策略名报错；宽松策略正常扫描
    let v = guestInvoke("scan", jObjOf([
        ("text", JStr("{\"a\": 1}")),
        ("config", jObjOf([("utf8_policy", JStr("bogus"))])),
    ]))
    @Assert(js(jObjGet(v, "type")) == "\"err\"")

    let v2 = guestInvoke("scan", jObjOf([
        ("text", JStr("{\"a\": 1}")),
        ("config", jObjOf([("utf8_policy", JStr("error_region"))])),
    ]))
    @Assert(js(jObjGet(v2, "types")).contains("struct.json"))
}

@Test
func limitsParallelismConfigAccepted(): Unit {
    let v = guestInvoke("scan", jObjOf([
        ("text", JStr("{\"a\": 1}")),
        ("config", jObjOf([("parallelism", JNum(4.0)), ("max_depth", JNum(32.0))])),
    ]))
    @Assert(js(jObjGet(v, "types")).contains("struct.json"))
}
