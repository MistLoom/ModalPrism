// 输入方式测试（对齐 RawPrism input_test.cj 能力域）：参数输入 / 分段输入 /
// 大文件输入 / 输入方式一致性 / 保留上限与物化不可用，经 Guest 与直接调用双路覆盖。
package rawprism_guest

import rawprism.*
import rawprism.core.*
import leaf_ability_sdk.*
import std.collection.*
import std.fs.*
import std.io.*
import std.unittest.*
import std.unittest.testmacro.*

@Test
func inputParameterMode(): Unit {
    let s = Session(Input("{\"a\": 1}"))
    s.scan()
    @Assert(s.getAll(["struct.json"]).size == 1)
    @Assert(s.input.fullyRetained())
}

@Test
func inputChunkedMode(): Unit {
    // 分段边界可落在任意位置（含 UTF-8 多字节字符内部）
    let src = StringChunkSource(["{\"na", "me\": \"仓", "颉 rp\"}"])
    let s = Session(Input(src))
    while (s.input.hasMore()) {}
    s.scan()
    @Assert(s.getAll(["struct.json"]).size == 1)
    @Assert(s.input.toText().contains("仓颉 rp"))
}

@Test
func inputFileMode(): Unit {
    let path = "guest_scan_tmp.txt"
    let f = File(Path(path), OpenMode.Write)
    let payload = "url https://f.io end {\"k\": [1,2,3]}"
    let bytes = payload.toArray()
    f.write(bytes)
    f.close()
    let s = Session(Input.fromFile(path, chunkSize: 8))
    while (s.input.hasMore()) {}
    s.scan()
    @Assert(s.getAll(["struct.json"]).size == 1)
    @Assert(s.getAll(["entity.url"]).size == 1)
    remove(Path(path))
}

@Test
func inputModeConsistency(): Unit {
    // 相同文本内容下，区域树与输入方式无关（8.4 / C2）
    let text = "{\"a\": 1} https://u.io tail"
    let s1 = Session(Input(text))
    s1.scan()
    let s2 = Session(Input(StringChunkSource([text])))
    while (s2.input.hasMore()) {}
    s2.scan()
    let r1 = s1.tree.ordered()
    let r2 = s2.tree.ordered()
    @Assert(r1.size == r2.size)
    for (i in 0..r1.size) {
        @Assert(spanEq(r1[i].span, r2[i].span))
        @Assert(r1[i].typeName == r2[i].typeName)
    }
}

@Test
func inputRetentionLimitAndDropped(): Unit {
    // 保留上限（4.4.6）：超出部分被丢弃，全局偏移保持稳定
    let chunks = ["0123456789", "abcdefghij", "{\"a\": 1}"]
    let src = StringChunkSource(chunks)
    let s = Session(Input(src, retentionLimit: 10))
    while (s.input.hasMore()) {}
    s.scan()
    @Assert(!s.input.fullyRetained())
    @Assert(s.input.droppedSpans().size > 0)
    // 已丢弃区间不可物化
    @Assert(s.input.isDropped(Span(0, 5)))
    // 命中区间仍可物化
    let hits = s.getAll(["struct.json"])
    @Assert(hits.size == 1)
}

func badUtf8Bytes(): Array<UInt8> {
    // 97='a', 255=非法起始字节, 98='b'
    let bytes = Array<UInt8>(3, repeat: 0)
    bytes[0] = 97
    bytes[1] = 255
    bytes[2] = 98
    bytes
}

@Test
func inputUtf8ErrorRegion(): Unit {
    // 宽松策略：非法 UTF-8 记错误区间，不使会话失败
    // a <bad-byte> b
    let bytes = badUtf8Bytes()
    let input = Input(bytes, policy: UTF8Policy.ErrorRegion)
    let s = Session(input)
    s.scan()
    @Assert(input.invalidUtf8Spans().size == 1)
}

@Test
func inputUtf8StrictThrows(): Unit {
    // 严格策略：构造时报告非法序列
    let bytes = badUtf8Bytes()
    var threw = false
    try {
        let _ = Input(bytes, policy: UTF8Policy.Strict)
    } catch (e: IllegalArgumentException) {
        threw = true
    }
    @Assert(threw)
}

@Test
func guestScanChunks(): Unit {
    let v = guestInvoke("scan_chunks", jObjOf([
        ("chunks", jArrOf([JStr("{\"na"), JStr("me\": 1}")])),
        ("selectors", jArrOf([JStr("struct.json")])),
    ]))
    let regions = jAsArr(jObjGet(v, "regions")) ?? ArrayList<JsonValue>()
    @Assert(regions.size == 1)
}

@Test
func guestScanChunksRetention(): Unit {
    let v = guestInvoke("scan_chunks", jObjOf([
        ("chunks", jArrOf([JStr("0123456789"), JStr("abcdefghij"), JStr("{\"a\": 1}")])),
        ("retention_limit", JNum(10.0)),
    ]))
    @Assert(js(jObjGet(v, "types")).contains("struct.json"))
}
