// 协议帧编解码测试。
package leaf_ability_sdk

import std.unittest.*
import std.unittest.testmacro.*

@Test
public func testFrameRoundTrip(): Unit {
    let frame = jObjOf([
        ("id", JStr("a1")),
        ("reply_to", JStr("h0")),
        ("body", jObjOf([
            ("type", JStr("hello")),
        ])),
    ])
    let encoded = encodeFrame(frame)
    @Assert(encoded.endsWith("\n"))
    let decoded = decodeFrame(encoded)
    @Assert(jAsStr(jObjGet(decoded, "id")) == Some("a1"))
    @Assert(jAsStr(jObjGet(decoded, "reply_to")) == Some("h0"))
    @Assert(jAsStr(jObjGet(jObjGet(decoded, "body"), "type")) == Some("hello"))
}

@Test
public func testDecodeEmptyFrameThrows(): Unit {
    var threw = false
    try {
        decodeFrame("   \n")
    } catch (e: SdkError) {
        threw = true
    }
    @Assert(threw)
}

@Test
public func testOkValueOk(): Unit {
    let frame = jObjOf([
        ("id", JStr("r1")),
        ("reply_to", JStr("q1")),
        ("body", okBody(JNum(42.0))),
    ])
    let v = okValue(frame)
    @Assert(jAsNum(v) == Some(42.0))
}

@Test
public func testOkValueMissingValueIsNull(): Unit {
    let frame = jObjOf([
        ("id", JStr("r2")),
        ("reply_to", JStr("q2")),
        ("body", jObjOf([("type", JStr("ok"))])),
    ])
    let v = okValue(frame)
    @Assert(jIsNull(v))
}

@Test
public func testOkValueErrThrowsWithKind(): Unit {
    let frame = jObjOf([
        ("id", JStr("r3")),
        ("reply_to", JStr("q3")),
        ("body", jObjOf([
            ("type", JStr("err")),
            ("error", jObjOf([
                ("kind", JStr("not_found")),
                ("message", JStr("没有这个东西")),
            ])),
        ])),
    ])
    var threw = false
    var kind = ""
    try {
        okValue(frame)
    } catch (e: SdkError) {
        threw = true
        kind = e.kind
    }
    @Assert(threw)
    @Assert(kind == "not_found")
}

@Test
public func testOkValueUnexpectedBodyThrows(): Unit {
    let frame = jObjOf([
        ("id", JStr("r4")),
        ("reply_to", JStr("q4")),
        ("body", jObjOf([("type", JStr("manifest"))])),
    ])
    var threw = false
    try {
        okValue(frame)
    } catch (e: SdkError) {
        threw = true
    }
    @Assert(threw)
}

@Test
public func testSchemaVersion(): Unit {
    @Assert(schemaVersion("cangjie@1") == "1")
    @Assert(schemaVersion("cj@1") == "1")
    @Assert(schemaVersion("1") == "1")
}
