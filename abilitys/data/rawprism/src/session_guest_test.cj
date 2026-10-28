// 会话查询与物化：getAll / first / typeCounts / walk / chainAt / intersecting /
// objectOf（对齐 RawPrism session_test.cj / object_test.cj 能力域），并经 Guest
// 验证 materialize 与 query_object 的物化输出。
package rawprism_guest

import rawprism.*
import rawprism.core.*
import rawprism.object.*
import leaf_ability_sdk.*
import std.collection.*
import std.unittest.*
import std.unittest.testmacro.*

@Test
func sessionGetAllAndFirst(): Unit {
    let text = "{\"a\": 1} mid {\"b\": 2} end"
    let s = Session(Input(text))
    s.scan()
    let all = s.getAll(["struct.json"])
    @Assert(all.size == 2)
    let f = s.first(["struct.json"])
    @Assert(f.isSome())
    if (let Some(r) <- f) {
        @Assert(r.span.start == 0)
    }
    @Assert(s.first(["struct.toml"]).isNone())
}

@Test
func sessionTypeCountsAndWalk(): Unit {
    let text = "https://u.io {\"a\": 1} plain\n"
    let s = Session(Input(text))
    s.scan()
    var total: Int64 = 0
    for ((k, v) in s.typeCounts()) {
        let _ = k
        total += v
    }
    @Assert(total == s.tree.size())
    let visited = Box()
    s.walk({ r: Region, d: Int64 =>
        let _ = r
        let _ = d
        visited.n += 1
        WalkControl.Continue
    })
    @Assert(visited.n == s.tree.size())
}

@Test
func sessionChainAtAndIntersecting(): Unit {
    let text = "# H\n{\"a\": 1}\n"
    let s = Session(Input(text))
    s.scan()
    // 点查询落在 json 区域上
    let chain = s.chainAt(5)
    @Assert(chain.size > 0)
    // 区间查询：与 json 区域相交
    let hits = s.intersecting(Span(4, 6))
    @Assert(hits.size > 0)
}

func optObjJson(o: ?Obj): String {
    match (o) {
        case Some(x) => js(objToJson(x))
        case None => "null"
    }
}

// 闭包内计数盒（仓颉 lambda 不能捕获 var）。
class Box {
    var n: Int64 = 0
}

@Test
func sessionObjectOfLazyCached(): Unit {
    let text = "{\"name\": \"rp\", \"tags\": [\"x\", \"y\"]}"
    let s = Session(Input(text))
    s.scan()
    let hits = s.getAll(["struct.json"])
    @Assert(hits.size == 1)
    let obj = s.objectOf(hits[0].region)
    @Assert(optObjJson(obj.get("name")) == "\"rp\"")
    let obj2 = s.objectOf(hits[0].region)
    @Assert(obj2.get("name").isSome())
}

@Test
func guestMaterializeTextAndValue(): Unit {
    let v = guestInvoke("materialize", jObjOf([
        ("text", JStr("{\"name\": \"rp\", \"n\": 3}")),
        ("selectors", jArrOf([JStr("struct.json")])),
        ("with_value", JBool(true)),
    ]))
    // 原文与输入一致（避免对转义文本做全等比较）
    let textOut = jAsStr(jObjGet(v, "text")) ?? ""
    @Assert(textOut.contains("name"))
    @Assert(textOut.contains("rp"))
    let value = jObjGet(v, "value")
    @Assert(js(jObjGet(value, "name")) == "\"rp\"")
    @Assert(js(jObjGet(value, "n")) == "3")
}

@Test
func guestMaterializeIndexOutOfBounds(): Unit {
    let v = guestInvoke("materialize", jObjOf([
        ("text", JStr("{\"a\": 1}")),
        ("selectors", jArrOf([JStr("struct.json")])),
        ("index", JNum(5.0)),
    ]))
    // 错误应答：type=err
    @Assert(js(jObjGet(v, "type")) == "\"err\"")
}

@Test
func guestMaterializeMultipleSelectors(): Unit {
    let v = guestInvoke("materialize", jObjOf([
        ("text", JStr("see https://u.io now")),
        ("selectors", jArrOf([JStr("entity.url")])),
    ]))
    @Assert(js(jObjGet(v, "type_id")) == "\"entity.url\"")
    @Assert(js(jObjGet(v, "text")) == "\"https://u.io\"")
}

@Test
func objectViewsAcrossFormats(): Unit {
    // 统一 Object 视图来源：JSON / YAML / CSV / TOML / XML / front matter
    let s1 = Session(Input("{\"a\": [1, 2]}"))
    s1.scan()
    let o1 = s1.objectOf(s1.getAll(["struct.json"])[0].region)
    @Assert(o1.at("a").isSome())

    let s2 = Session(Input("name: rp\nver: 2\n"))
    s2.scan()
    let o2 = s2.objectOf(s2.getAll(["struct.yaml"])[0].region)
    @Assert(optObjJson(o2.get("name")) == "\"rp\"")

    let s3 = Session(Input("id,name\n1,rp\n"))
    s3.scan()
    let hits3 = s3.getAll(["struct.csv"])
    @Assert(hits3.size >= 1)

    let s4 = Session(Input("[t]\nx = 1\n"))
    s4.scan()
    @Assert(s4.getAll(["struct.toml"]).size >= 1)

    let s5 = Session(Input("<r><v>9</v></r>"))
    s5.scan()
    @Assert(s5.getAll(["struct.xml"]).size >= 1)
}

@Test
func objectWalkAndEnumerate(): Unit {
    let s = Session(Input("{\"a\": 1, \"b\": {\"c\": true}}"))
    s.scan()
    let root = s.objectOf(s.getAll(["struct.json"])[0].region)
    let count = Box()
    let _ = root.walk({ _o: Obj =>
        count.n += 1
        WalkControl.Continue
    })
    @Assert(count.n >= 4)
    let entries = root.entries()
    @Assert(entries.size == 2)
}
