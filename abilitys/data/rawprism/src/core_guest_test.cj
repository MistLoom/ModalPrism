// 核心类型与选择器：Span / TypeId 解析与匹配（对齐 RawPrism core_test.cj 能力域），
// 并经 Guest 验证 scan 的区域与类型计数输出。
package rawprism_guest

import rawprism.core.*
import rawprism.*
import leaf_ability_sdk.*
import std.collection.*
import std.unittest.*
import std.unittest.testmacro.*

@Test
func guestSpanBasics(): Unit {
    let s = Span(2, 10)
    @Assert(s.length == 8)
    @Assert(s.contains(2))
    @Assert(!s.contains(10))
    @Assert(s.contains(Span(3, 9)))
    @Assert(s.intersects(Span(9, 12)))
    @Assert(Span(5, 5).isEmpty())
}

@Test
func guestTypeIdParsing(): Unit {
    let t1 = TypeId("struct.json")
    @Assert(t1.text == "struct.json")
    @Assert(t1.category == "struct")
    let t2 = TypeId("markup.html#text")
    @Assert(t2.text == "markup.html#text")
    @Assert(t2.sub == Some("text"))
    let t3 = TypeId("code.block[lang=json]")
    @Assert(t3.text == "code.block")
    @Assert(t3.params.get("lang") == Some("json"))
}

@Test
func guestTypeIdSelectorMatching(): Unit {
    @Assert(TypeId("text.md.paragraph").matches("text.md"))
    @Assert(TypeId("text.md.paragraph").matches("text.*"))
    @Assert(!TypeId("struct.json").matches("text.*"))
    @Assert(TypeId("markup.html#text").matches("markup.html#text"))
    @Assert(!TypeId("markup.html").matches("markup.html#text"))
    @Assert(TypeId("code.block[lang=json]").matches("code.block[lang=json]"))
    @Assert(!TypeId("code.block[lang=yaml]").matches("code.block[lang=json]"))
}

@Test
func guestScanRegionsAndCounts(): Unit {
    let text = "# note\n{\"a\": 1} https://example.com a@b.io\n"
    let v = guestInvoke("scan", jObjOf([
        ("text", JStr(text)),
        ("selectors", jArrOf([JStr("struct.json"), JStr("entity.url"), JStr("entity.email")])),
    ]))
    // 区域信息：type_id / start / end / confidence
    let regions = jObjGet(v, "regions")
    @Assert(!jIsNull(regions))
    @Assert(jAsArr(regions).isSome())
    // 类型计数含三种类型
    let types = js(jObjGet(v, "types"))
    @Assert(types.contains("struct.json"))
    @Assert(types.contains("entity.url"))
    @Assert(types.contains("entity.email"))
    // 首个区域是 json，区间 [7, 16)
    @Assert(js(v).contains("struct.json"))
    @Assert(js(v).contains("\"start\":7"))
}

@Test
func guestScanEmptySelectorsReturnsAll(): Unit {
    let v = guestInvoke("scan", jObjOf([
        ("text", JStr("{\"k\": 1} tail https://x.io")),
    ]))
    let types = js(jObjGet(v, "types"))
    @Assert(types.contains("struct.json"))
    @Assert(types.contains("entity.url"))
}

@Test
func guestManifestDeclaresMethods(): Unit {
    let body = guestManifest()
    let manifest = jObjGet(body, "manifest")
    @Assert(jAsStr(jObjGet(manifest, "id")) == Some("rawprism"))
    let methods = jAsArr(jObjGet(manifest, "methods")) ?? ArrayList<JsonValue>()
    var ids = ArrayList<String>()
    for (m in methods) {
        if (let Some(id) <- jAsStr(jObjGet(m, "id"))) {
            ids.add(id)
        }
    }
    @Assert(ids.contains("scan"))
    @Assert(ids.contains("scan_file"))
    @Assert(ids.contains("scan_chunks"))
    @Assert(ids.contains("materialize"))
    @Assert(ids.contains("query_object"))
    @Assert(ids.contains("custom_detect"))
    @Assert(ids.contains("scan_async"))
    @Assert(ids.contains("scan_concurrent"))
    @Assert(ids.contains("scan_with_cancel"))
}
