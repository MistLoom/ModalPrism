// 自定义检测器与获取器（对齐 RawPrism detector 注册能力 + macro_test.cj 能力域）：
// Guest 侧有限自定义检测器（起止标记），直接调用覆盖 Detector / Getter<T> /
// GetterRegistry / @Detector 宏 / dependsOn 副产物登记。
package rawprism_guest

import rawprism.*
import rawprism.core.*
import rawprism.object.*
import rawprism_macros.*
import leaf_ability_sdk.*
import std.collection.*
import std.unittest.*
import std.unittest.testmacro.*

// 直接调用的自定义 Getter：识别工单号 TKT-123。
@Detector[typeId = "entity.ticket", within = ["text.*"]]
class GuestTicketId <: Getter<String> {
    public func detect(scope: Span, ctx: DetectContext): Unit {
        let data = ctx.session.inputBytes()
        let key = "TKT-".toArray()
        var i = scope.start
        while (i + key.size <= scope.end) {
            var hit = true
            for (k in 0..key.size) {
                if (data[i + k] != key[k]) {
                    hit = false
                    break
                }
            }
            if (hit) {
                var e = i + key.size
                while (e < scope.end && data[e] >= 48 && data[e] <= 57) {
                    e += 1
                }
                if (e > i + key.size) {
                    let _ = ctx.emit(Span(i, e), typeId, 1.0, HashMap<String, String>())
                    i = e
                    continue
                }
            }
            i += 1
        }
    }
    public func materialize(region: Region, ctx: MaterializeContext): String {
        ctx.text(region.span)
    }
}

@Test
func guestCustomDetectStartEndMarkers(): Unit {
    let v = guestInvoke("custom_detect", jObjOf([
        ("text", JStr("a [[o1]] b [[o2]] c")),
        ("type_id", JStr("custom.order")),
        ("start_marker", JStr("[[")),
        ("end_marker", JStr("]]")),
    ]))
    let regions = jAsArr(jObjGet(v, "regions")) ?? ArrayList<JsonValue>()
    @Assert(regions.size == 2)
    let types = js(jObjGet(v, "types"))
    @Assert(types.contains("custom.order"))
}

@Test
func guestCustomDetectLineEndDefault(): Unit {
    // end_marker 缺省以行尾闭合
    let v = guestInvoke("custom_detect", jObjOf([
        ("text", JStr("tok ABC\ntok DEF\n")),
        ("type_id", JStr("custom.tok")),
        ("start_marker", JStr("tok ")),
    ]))
    let regions = jAsArr(jObjGet(v, "regions")) ?? ArrayList<JsonValue>()
    @Assert(regions.size == 2)
}

@Test
func guestCustomDetectRequiresTypeId(): Unit {
    let v = guestInvoke("custom_detect", jObjOf([
        ("text", JStr("x [[y]] z")),
        ("start_marker", JStr("[[")),
    ]))
    @Assert(js(jObjGet(v, "type")) == "\"err\"")
}

@Test
func registryRegisterAndLookup(): Unit {
    let registry = GetterRegistry()
    @Assert(registry.register(GuestTicketId()))
    // 重复注册同一类型被拒（保持单例）
    @Assert(registry.register(GuestTicketId()))
    @Assert(registry.detectorFor("entity.ticket").isSome())
    @Assert(registry.detectorFor("no.such").isNone())
}

@Test
func customGetterEndToEndDirect(): Unit {
    let registry = GetterRegistry()
    let _ = registry.register(GuestTicketId())
    let text = "ref TKT-42 in plain text"
    let s = Session(Input(text), registry: registry)
    s.scan()
    let hits = s.getAll(["entity.ticket"])
    @Assert(hits.size == 1)
    @Assert(hits[0].region.typeName == "entity.ticket")
}

// 依赖排序用检测器（9.5）。
class GuestDepA <: Detector {
    public prop typeId: TypeId { get() { TypeId("dep.a") } }
    public prop within: Array<String> { get() { [] } }
    public prop dependsOn: Array<TypeId> { get() { [TypeId("dep.b")] } }
    public prop authoritative: Array<TypeId> { get() { [] } }
    public func detect(_scope: Span, _ctx: DetectContext): Unit {}
}

class GuestDepB <: Detector {
    public prop typeId: TypeId { get() { TypeId("dep.b") } }
    public prop within: Array<String> { get() { [] } }
    public prop dependsOn: Array<TypeId> { get() { [] } }
    public prop authoritative: Array<TypeId> { get() { [] } }
    public func detect(_scope: Span, _ctx: DetectContext): Unit {}
}

@Test
func customDetectorDependsOnOrdering(): Unit {
    let registry = GetterRegistry()
    let _ = registry.register(GuestDepA())
    let _ = registry.register(GuestDepB())
    let ordered = registry.ordered()
    @Assert(ordered.size == 2)
    @Assert(ordered[0].typeId.text == "dep.b")
}

@Test
func customGetterMaterialize(): Unit {
    // 惰性物化（C6）：materialize 按需取原文
    let registry = GetterRegistry()
    let _ = registry.register(GuestTicketId())
    let s = Session(Input("TKT-7 end"), registry: registry)
    s.scan()
    let hits = s.getAll(["entity.ticket"])
    @Assert(hits.size == 1)
    @Assert(s.objectOf(hits[0].region).rawText() == "TKT-7")
}
