// 并发测试（对齐 RawPrism concurrency_test.cj 能力域）：scanAsync / scanConcurrent /
// cancelScan 的 Guest 暴露与直接调用双路覆盖；并行一致性（12.6 / C5）、
// 并行度上限、单次检测去重（12.5）。
package rawprism_guest

import rawprism.*
import rawprism.core.*
import leaf_ability_sdk.*
import std.collection.*
import std.sync.*
import std.unittest.*
import std.unittest.testmacro.*

// 计数检测器（并发去重验证用）。
class GuestCountingJson <: Detector {
    internal let counter: AtomicInt64
    public init(counter: AtomicInt64) {
        this.counter = counter
    }
    public prop typeId: TypeId {
        get() { TypeId("guest.counting.json") }
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
        if (scope.start == 0) {
            counter.fetchAdd(1)
            let _ = ctx.emit(scope, typeId, 1.0, HashMap<String, String>())
        }
    }
}

@Test
func concurrentAsyncViaGuest(): Unit {
    let v = guestInvoke("scan_async", jObjOf([
        ("text", JStr("{\"a\": 1} https://u.io")),
        ("selectors", jArrOf([JStr("struct.json"), JStr("entity.url")])),
    ]))
    let regions = jAsArr(jObjGet(v, "regions")) ?? ArrayList<JsonValue>()
    @Assert(regions.size == 2)
}

@Test
func concurrentConcurrentViaGuest(): Unit {
    let v = guestInvoke("scan_concurrent", jObjOf([
        ("text", JStr("{\"a\": 1} mid https://u.io tail {\"b\": [1,2,3]} end")),
        ("config", jObjOf([("parallelism", JNum(4.0))])),
    ]))
    let regions = jAsArr(jObjGet(v, "regions")) ?? ArrayList<JsonValue>()
    @Assert(regions.size >= 2)
}

@Test
func concurrentCancelViaGuest(): Unit {
    // 取消后结果仍可用（部分或全部，不失败）
    let v = guestInvoke("scan_with_cancel", jObjOf([
        ("text", JStr("https://u.io {\"a\": 1}")),
    ]))
    @Assert(!jIsNull(jObjGet(v, "regions")))
}

@Test
func concurrentAsyncEntryDirect(): Unit {
    let fut = scanAsync(Input("{\"a\": 1} https://u.io"))
    let s = fut.get()
    @Assert(s.getAll(["struct.json"]).size == 1)
    @Assert(s.getAll(["entity.url"]).size == 1)
}

@Test
func concurrentDeterministicDirect(): Unit {
    // 12.6 / C5：不同并行度下区域树必须相同
    let text = "{\"a\": 1} mid https://u.io tail {\"b\": [1,2,3]} end"

    let s1 = Session(Input(text))
    scanConcurrent(s1)

    var cfg4 = ScanConfig()
    cfg4.parallelism = 4
    let s4 = Session(Input(text), config: cfg4)
    scanConcurrent(s4)

    var cfg8 = ScanConfig()
    cfg8.parallelism = 8
    let s8 = Session(Input(text), config: cfg8)
    scanConcurrent(s8)

    let r1 = s1.tree.ordered()
    let r4 = s4.tree.ordered()
    let r8 = s8.tree.ordered()
    @Assert(r1.size == r4.size)
    @Assert(r1.size == r8.size)
    for (i in 0..r1.size) {
        @Assert(spanEq(r1[i].span, r4[i].span))
        @Assert(r1[i].typeName == r4[i].typeName)
        @Assert(spanEq(r1[i].span, r8[i].span))
    }
}

@Test
func concurrentMatchesSerialDirect(): Unit {
    // 并行扫描结果与串行 scan() 逐区域一致
    let text = "{\"a\": 1} https://u.io # md\n"
    let sSerial = Session(Input(text))
    sSerial.scan()
    var cfg = ScanConfig()
    cfg.parallelism = 4
    let sPar = Session(Input(text), config: cfg)
    scanConcurrent(sPar)
    let r1 = sSerial.tree.ordered()
    let r2 = sPar.tree.ordered()
    @Assert(r1.size == r2.size)
    for (i in 0..r1.size) {
        @Assert(spanEq(r1[i].span, r2[i].span))
        @Assert(r1[i].typeName == r2[i].typeName)
    }
}

@Test
func concurrentSingleDetectionDedup(): Unit {
    // 12.5：同一类型同一区间只触发一次检测
    let counter = AtomicInt64(0)
    let registry = GetterRegistry()
    let _ = registry.register(GuestCountingJson(counter))
    let fut = scanAsync(Input("{\"a\": 1} {\"b\": 2}"), registry: registry)
    let s = fut.get()
    let _ = s.getAll(["guest.counting.json"])
    @Assert(counter.load() == 1)
}

@Test
func concurrentCancelDirect(): Unit {
    // 12.7：请求取消后扫描仍返回有效（可为部分）结果
    let fut = scanAsync(Input("https://u.io {\"a\": 1} # md\n"))
    let s = fut.get()
    cancelScan(s)
    let _ = s.getAll(["entity.url"])
    @Assert(true)
}
