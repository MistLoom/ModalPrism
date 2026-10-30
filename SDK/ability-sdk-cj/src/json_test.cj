// JSON 编解码测试。
package leaf_ability_sdk

import std.collection.ArrayList

import std.unittest.*
import std.unittest.testmacro.*

@Test
public func testParseScalars(): Unit {
    @Assert(jIsNull(jsonParse("null")))
    @Assert(jEquals(jsonParse("true"), JBool(true)))
    @Assert(jAsNum(jsonParse("-3.5")) == Some(-3.5))
    @Assert(jAsStr(jsonParse("\"hi\"")) == Some("hi"))
}

@Test
public func testParseNested(): Unit {
    let v = jsonParse("{\"a\":[1,{\"b\":\"x\"}],\"c\":null}")
    let a = jObjGet(v, "a")
    let arr = match (jAsArr(a)) {
        case Some(l) => l
        case None => @Assert(false); ArrayList<JsonValue>()
    }
    @Assert(arr.size == 2)
    @Assert(jAsNum(arr[0]) == Some(1.0))
    @Assert(jAsStr(jObjGet(arr[1], "b")) == Some("x"))
    @Assert(jIsNull(jObjGet(v, "c")))
}

@Test
public func testParseEscapes(): Unit {
    let v = jsonParse("\"a\\n\\t\\\"\\\\\\u4e2d\"")
    @Assert(jAsStr(v) == Some("a\n\t\"\\中"))
}

@Test
public func testParseSurrogatePair(): Unit {
    // U+1D11E 𝄞
    let v = jsonParse("\"\\ud834\\udd1e\"")
    @Assert(jAsStr(v) == Some("𝄞"))
}

@Test
public func testSerializeRoundTrip(): Unit {
    let v = jsonParse("{\"k\":[1,\"s\",true,null],\"esc\":\"a\\nb\"}")
    let text = v.toJsonString()
    let back = jsonParse(text)
    @Assert(back.toJsonString() == text)
}

@Test
public func testSerializeEscapesControlChars(): Unit {
    let s = jObjOf([("a", JStr("x\ny"))])
    let text = s.toJsonString()
    @Assert(text == "{\"a\":\"x\\ny\"}")
}

@Test
public func testParseInvalidThrows(): Unit {
    var threw = false
    try {
        jsonParse("{\"a\":}")
    } catch (e: SdkError) {
        threw = true
    }
    @Assert(threw)
}
