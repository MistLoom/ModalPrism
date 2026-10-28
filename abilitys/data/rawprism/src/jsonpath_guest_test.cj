// JSONPath（RFC 9535）与 Object 查询（对齐 RawPrism jsonpath_test.cj 能力域），
// 经 Guest query_object 与直接调用双路覆盖。
package rawprism_guest

import rawprism.*
import rawprism.core.*
import rawprism.object.*
import leaf_ability_sdk.*
import std.collection.*
import std.unittest.*
import std.unittest.testmacro.*

func jsonHits(text: String, query: String): ArrayList<JsonValue> {
    let v = guestInvoke("query_object", jObjOf([
        ("text", JStr(text)),
        ("selectors", jArrOf([JStr("struct.json")])),
        ("query", JStr(query)),
    ]))
    return jAsArr(jObjGet(v, "hits")) ?? ArrayList<JsonValue>()
}

@Test
func jsonpathRootAndChild(): Unit {
    let hits = jsonHits("{\"name\": \"rp\", \"ver\": 2}", "$.name")
    @Assert(hits.size == 1)
    @Assert(js(hits[0]) == "\"rp\"")
}

@Test
func jsonpathWildcardAndDescendant(): Unit {
    let text = "{\"a\": {\"v\": 1}, \"b\": {\"v\": 2}}"
    let hits = jsonHits(text, "$..v")
    @Assert(hits.size == 2)
    let hits2 = jsonHits(text, "$.*")
    @Assert(hits2.size == 2)
}

@Test
func jsonpathArrayIndexAndSlice(): Unit {
    let text = "{\"tags\": [\"x\", \"y\", \"z\"]}"
    @Assert(jsonHits(text, "$.tags[0]").size == 1)
    @Assert(jsonHits(text, "$.tags[*]").size == 3)
}

@Test
func jsonpathFilter(): Unit {
    let text = "{\"items\": [{\"n\": 1}, {\"n\": 5}, {\"n\": 9}]}"
    let hits = jsonHits(text, "$.items[?@.n > 4]")
    @Assert(hits.size == 2)
}

@Test
func jsonpathNoMatchReturnsEmpty(): Unit {
    @Assert(jsonHits("{\"a\": 1}", "$.missing").size == 0)
}

@Test
func jsonpathDirectApi(): Unit {
    let s = Session(Input("{\"a\": {\"b\": [10, 20, 30]}}"))
    s.scan()
    let root = s.objectOf(s.getAll(["struct.json"])[0].region)
    let hits = jsonPath(root, "$.a.b[*]")
    @Assert(hits.size == 3)
    // objPath：单点直达（自动补 $ 前缀）
    let one = objPath(root, "a.b[1]")
    @Assert(one.isSome())
    if (let Some(v) <- one) {
        @Assert(js(objToJson(v)) == "20")
    }
}

@Test
func jsonpathParseSegments(): Unit {
    let segs = parsePath("$.a[0].*")
    @Assert(segs.size == 3)
}

@Test
func jsonpathOnOtherFormats(): Unit {
    // JSONPath 同样作用于 YAML / XML / CSV 派生的 Object
    let s = Session(Input("name: rp\nitems:\n  - 1\n  - 2\n"))
    s.scan()
    let root = s.objectOf(s.getAll(["struct.yaml"])[0].region)
    let hits = jsonPath(root, "$.items[*]")
    @Assert(hits.size == 2)
}
