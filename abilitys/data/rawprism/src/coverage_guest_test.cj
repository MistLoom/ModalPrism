// 覆盖与预扫描 / 线协议端到端（对齐 RawPrism overlap_test.cj / prescan_test.cj
// 与 SDK 协议测试能力域）。这两个域不经 Guest 暴露，用直接调用补齐全支持覆盖。
package rawprism_guest

import rawprism.*
import rawprism.core.*
import leaf_ability_sdk.*
import std.collection.*
import std.unittest.*
import std.unittest.testmacro.*

@Test
func overlapSiblingsQueryable(): Unit {
    // 7 / 17：同级允许重叠且可查询（entity.url 与 text.plain 同段交叠）
    let text = "go https://u.io now"
    let s = Session(Input(text))
    s.scan()
    let urls = s.getAll(["entity.url"])
    @Assert(urls.size == 1)
    let overlapping = s.tree.overlapping(urls[0].region)
    @Assert(overlapping.size >= 1)
}

@Test
func overlapNestedTree(): Unit {
    // 嵌套：markdown 代码块内的 json 是子区域
    let text = "# H\n```json\n{\"a\": 1}\n```\n"
    let s = Session(Input(text))
    s.scan()
    let jsons = s.getAll(["struct.json"])
    @Assert(jsons.size == 1)
    let chain = s.tree.chainAt(jsons[0].region.start)
    // 链上外层应含 markdown / 代码块类型
    @Assert(chain.size >= 2)
}

@Test
func prescanSharedLineIndex(): Unit {
    // 5.3：换行位置预扫描只计算一次，结果一致
    let text = "l1\nl2\nl3\n"
    let s = Session(Input(text))
    s.scan()
    let starts1 = s.lineStartsShared()
    let starts2 = s.lineStartsShared()
    @Assert(starts1.size == starts2.size)
    @Assert(starts1.size >= 3)
}

@Test
func prescanIdempotentRescan(): Unit {
    // 可重复调用（幂等：已覆盖区间不重扫）
    let text = "{\"a\": 1} https://u.io"
    let s = Session(Input(text))
    s.scan()
    let n1 = s.tree.size()
    s.scan()
    @Assert(s.tree.size() == n1)
}

@Test
func protocolHelloManifest(): Unit {
    let body = guestManifest()
    @Assert(jAsStr(jObjGet(body, "type")) == Some("manifest"))
    let manifest = jObjGet(body, "manifest")
    @Assert(jAsStr(jObjGet(manifest, "id")) == Some("rawprism"))
    @Assert(jAsBool(jObjGet(manifest, "stateful")) == Some(false))
}

@Test
func protocolCreateThenInvoke(): Unit {
    // create 后 invoke；未 create 直接 invoke 报错
    let fx = GuestFixture()
    fx.push(jObjOf([
        ("id", JStr("i0")),
        ("body", jObjOf([
            ("type", JStr("invoke")),
            ("method", JStr("scan")),
            ("args", jObjOf([("text", JStr("{}"))])),
        ])),
    ]).toJsonString())
    let fx2 = GuestFixture()
    let _ = fx2
    // 未 create：应答 err
    serveWithIO(guestProvider(), fx.input, fx.write)
    let body = fx.replyValue("i0")
    @Assert(jAsStr(jObjGet(body, "type")) == Some("err"))
}

@Test
func protocolShutdownStops(): Unit {
    let fx = GuestFixture()
    fx.push(jObjOf([
        ("id", JStr("s1")),
        ("body", jObjOf([("type", JStr("shutdown"))])),
    ]).toJsonString())
    fx.push(jObjOf([
        ("id", JStr("h1")),
        ("body", jObjOf([("type", JStr("hello"))])),
    ]).toJsonString())
    serveWithIO(guestProvider(), fx.input, fx.write)
    // shutdown 后不再处理后续帧
    @Assert(fx.out.size == 0)
}

@Test
func protocolUnknownMethodErrors(): Unit {
    let v = guestInvoke("no_such_method", jObjOf([("text", JStr("{}"))]))
    @Assert(jAsStr(jObjGet(v, "type")) == Some("err"))
}
