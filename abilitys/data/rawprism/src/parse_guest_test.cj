// 格式解析测试（对齐 RawPrism parse_test.cj 能力域）：
// JSON / NDJSON / YAML / TOML / CSV / XML / HTML / Markdown / 纯文本 的 TypeId
// 与区域识别，经 Guest scan 方法与直接调用 RawPrism 双路覆盖。
package rawprism_guest

import rawprism.*
import rawprism.core.*
import rawprism.object.*
import leaf_ability_sdk.*
import std.collection.*
import std.unittest.*
import std.unittest.testmacro.*

func typeTextsOf(v: JsonValue): ArrayList<String> {
    let out = ArrayList<String>()
    let types = jAsArr(jObjGet(v, "types")) ?? ArrayList<JsonValue>()
    for (t in types) {
        if (let Some(id) <- jAsStr(jObjGet(t, "id"))) {
            out.add(id)
        }
    }
    out
}

func hasType(v: JsonValue, id: String): Bool {
    for (t in typeTextsOf(v)) {
        if (t == id) {
            return true
        }
    }
    false
}

@Test
func parseJsonViaGuest(): Unit {
    let v = guestInvoke("scan", jObjOf([("text", JStr("{\"name\": \"rp\", \"n\": 2}"))]))
    @Assert(hasType(v, "struct.json"))
    @Assert(!hasType(v, "struct.ndjson"))
}

@Test
func parseNdjsonViaGuest(): Unit {
    let v = guestInvoke("scan", jObjOf([("text", JStr("{\"a\": 1}\n{\"a\": 2}\n"))]))
    @Assert(hasType(v, "struct.ndjson"))
}

@Test
func parseYamlViaGuest(): Unit {
    let v = guestInvoke("scan", jObjOf([("text", JStr("name: rp\nversion: 1\n"))]))
    @Assert(hasType(v, "struct.yaml"))
}

@Test
func parseTomlViaGuest(): Unit {
    let v = guestInvoke("scan", jObjOf([("text", JStr("[pkg]\nname = \"rp\"\n"))]))
    @Assert(hasType(v, "struct.toml"))
}

@Test
func parseCsvViaGuest(): Unit {
    let v = guestInvoke("scan", jObjOf([("text", JStr("id,name\n1,rp\n2,cj\n"))]))
    @Assert(hasType(v, "struct.csv"))
}

@Test
func parseXmlViaGuest(): Unit {
    let v = guestInvoke("scan", jObjOf([("text", JStr("<root><item id=\"1\">a</item></root>"))]))
    @Assert(hasType(v, "struct.xml"))
}

@Test
func parseHtmlViaGuest(): Unit {
    let text = "<html><head><title>t</title></head><body><p>hello</p></body></html>"
    let v = guestInvoke("scan", jObjOf([("text", JStr(text))]))
    @Assert(hasType(v, "markup.html"))
}

@Test
func parseMarkdownViaGuest(): Unit {
    let text = "# Title\n\npara text\n\n```json\n{\"a\": 1}\n```\n"
    let v = guestInvoke("scan", jObjOf([("text", JStr(text))]))
    @Assert(hasType(v, "text.md"))
    let subs = typeTextsOf(v)
    var sawSub = false
    for (t in subs) {
        if (t.startsWith("text.md.")) {
            sawSub = true
        }
    }
    @Assert(sawSub)
}

@Test
func parsePlainViaGuest(): Unit {
    // 纯文本输入：粗分类回退为 text.plain（无独立检测器，不产出区域），
    // 与 RawPrism session_test.coarseClassifyPlain 一致。
    let s = Session(Input("just plain words here"))
    s.scan()
    @Assert(s.coarseRootType().text == "text.plain")
    @Assert(s.getAll(["struct.json"]).size == 0)
    // 混合输入：结构化区域照常识别
    let v2 = guestInvoke("scan", jObjOf([("text", JStr("plain words\n{\"a\": 1}\n"))]))
    @Assert(hasType(v2, "struct.json"))
}

@Test
func parseEntitiesViaGuest(): Unit {
    let v = guestInvoke("scan", jObjOf([("text", JStr("see https://example.com/a?b=1 and mail a.b@c.io"))]))
    @Assert(hasType(v, "entity.url"))
    @Assert(hasType(v, "entity.email"))
}

@Test
func parseConsistencyDirectVsGuest(): Unit {
    // 直接调用与 Guest 调用对同一文本产出相同类型集合
    let text = "{\"a\": 1} https://u.io # md\n"
    let s = Session(Input(text))
    s.scan()
    var direct = HashSet<String>()
    for (r in s.tree.ordered()) {
        direct.add(r.typeName)
    }
    let v = guestInvoke("scan", jObjOf([("text", JStr(text))]))
    for (t in typeTextsOf(v)) {
        @Assert(direct.contains(t))
    }
}

@Test
func parseNestedSelectors(): Unit {
    // 选择器前缀匹配：text.* 命中 text.md 及其子类型
    let text = "# H\nbody\n"
    let v = guestInvoke("scan", jObjOf([
        ("text", JStr(text)),
        ("selectors", jArrOf([JStr("text.*")])),
    ]))
    for (t in typeTextsOf(v)) {
        @Assert(TypeId(t).matches("text.*"))
    }
}
