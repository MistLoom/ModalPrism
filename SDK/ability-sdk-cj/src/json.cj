// JSON 值模型与解析/序列化（无第三方依赖，风格对齐 TS/Python SDK 的 yaml-lite）。
package leaf_ability_sdk

import std.collection.ArrayList
import std.convert.*

public enum JsonValue {
    | JNull
    | JBool(Bool)
    | JNum(Float64)
    | JStr(String)
    | JArr(ArrayList<JsonValue>)
    | JObj(ArrayList<(String, JsonValue)>)

    public func toJsonString(): String {
        let sb = StringBuilder()
        writeJson(this, sb)
        return sb.toString()
    }
}

public func writeJson(v: JsonValue, sb: StringBuilder): Unit {
    match (v) {
        case JNull => sb.append("null")
        case JBool(b) => sb.append(if (b) { "true" } else { "false" })
        case JNum(n) => sb.append(numToString(n))
        case JStr(s) => writeJsonString(s, sb)
        case JArr(items) =>
            sb.append("[")
            for (i in 0..items.size) {
                if (i > 0) {
                    sb.append(",")
                }
                writeJson(items[i], sb)
            }
            sb.append("]")
        case JObj(pairs) =>
            sb.append("{")
            for (i in 0..pairs.size) {
                if (i > 0) {
                    sb.append(",")
                }
                writeJsonString(pairs[i][0], sb)
                sb.append(":")
                writeJson(pairs[i][1], sb)
            }
            sb.append("}")
    }
}

func numToString(n: Float64): String {
    if (n >= -1.0e15 && n <= 1.0e15 && n == Float64(Int64(n))) {
        return Int64(n).toString()
    }
    return n.toString()
}

func writeJsonString(s: String, sb: StringBuilder): Unit {
    sb.append("\"")
    for (r in s.runes()) {
        if (r == r'\"') {
            sb.append("\\\"")
        } else if (r == r'\\') {
            sb.append("\\\\")
        } else if (r == r'\n') {
            sb.append("\\n")
        } else if (r == r'\r') {
            sb.append("\\r")
        } else if (r == r'\t') {
            sb.append("\\t")
        } else if (r == r'\b') {
            sb.append("\\b")
        } else if (r == r'\f') {
            sb.append("\\f")
        } else if (UInt32(r) < 0x20) {
            sb.append("\\u")
            let code = UInt32(r)
            for (i in 0..4) {
                let shift = (3 - i) * 4
                let digit = (code >> shift) & 0xF
                sb.append(hexDigit(digit))
            }
        } else {
            sb.append(r.toString())
        }
    }
    sb.append("\"")
}

func hexDigit(d: UInt32): String {
    let chars = "0123456789abcdef"
    return chars[Int64(d)..Int64(d + 1)]
}

// ---------- 访问辅助 ----------

public func jIsNull(v: JsonValue): Bool {
    match (v) {
        case JNull => true
        case _ => false
    }
}

// 通过序列化文本比较两个 JSON 值是否相等。
public func jEquals(a: JsonValue, b: JsonValue): Bool {
    return a.toJsonString() == b.toJsonString()
}

public func jObjGet(v: JsonValue, key: String): JsonValue {
    match (v) {
        case JObj(pairs) =>
            for (p in pairs) {
                if (p[0] == key) {
                    return p[1]
                }
            }
            return JNull
        case _ => return JNull
    }
}

public func jAsStr(v: JsonValue): ?String {
    match (v) {
        case JStr(s) => Some(s)
        case _ => None
    }
}

public func jAsNum(v: JsonValue): ?Float64 {
    match (v) {
        case JNum(n) => Some(n)
        case _ => None
    }
}

public func jAsBool(v: JsonValue): ?Bool {
    match (v) {
        case JBool(b) => Some(b)
        case _ => None
    }
}

public func jAsArr(v: JsonValue): ?ArrayList<JsonValue> {
    match (v) {
        case JArr(a) => Some(a)
        case _ => None
    }
}

public func jArrOf(items: Array<JsonValue>): JsonValue {
    let list = ArrayList<JsonValue>()
    for (it in items) {
        list.add(it)
    }
    return JArr(list)
}

public func jObjOf(pairs: Array<(String, JsonValue)>): JsonValue {
    let list = ArrayList<(String, JsonValue)>()
    for (p in pairs) {
        list.add(p)
    }
    return JObj(list)
}

// ---------- 解析器 ----------

public func runeArray(s: String): Array<Rune> {
    let list = ArrayList<Rune>()
    for (r in s.runes()) {
        list.add(r)
    }
    return list.toArray()
}

public func jsonParse(text: String): JsonValue {
    let p = JsonParser(runeArray(text))
    p.skipWs()
    let v = p.parseValue()
    p.skipWs()
    if (p.pos < p.rs.size) {
        throw SdkError("JSON 尾部有多余字符", "failed")
    }
    return v
}

internal class JsonParser {
    let rs: Array<Rune>
    var pos: Int64

    init(rs: Array<Rune>) {
        this.rs = rs
        this.pos = 0
    }

    func skipWs(): Unit {
        while (pos < rs.size) {
            let r = rs[pos]
            if (r == r' ' || r == r'\t' || r == r'\n' || r == r'\r') {
                pos += 1
            } else {
                break
            }
        }
    }

    func peek(): Rune {
        return rs[pos]
    }

    func expect(r: Rune): Unit {
        if (pos >= rs.size || rs[pos] != r) {
            throw SdkError("JSON 语法错误: 期望 '${r}'", "failed")
        }
        pos += 1
    }

    func parseValue(): JsonValue {
        skipWs()
        if (pos >= rs.size) {
            throw SdkError("JSON 意外结束", "failed")
        }
        let r = peek()
        if (r == r'{') {
            return parseObj()
        }
        if (r == r'[') {
            return parseArr()
        }
        if (r == r'"') {
            return JStr(parseStr())
        }
        if (r == r't') {
            expectWord("true")
            return JBool(true)
        }
        if (r == r'f') {
            expectWord("false")
            return JBool(false)
        }
        if (r == r'n') {
            expectWord("null")
            return JNull
        }
        return parseNum()
    }

    func expectWord(w: String): Unit {
        let wr = runeArray(w)
        for (i in 0..wr.size) {
            if (pos >= rs.size || rs[pos] != wr[i]) {
                throw SdkError("JSON 语法错误: 期望 ${w}", "failed")
            }
            pos += 1
        }
    }

    func parseObj(): JsonValue {
        expect(r'{')
        let pairs = ArrayList<(String, JsonValue)>()
        skipWs()
        if (pos < rs.size && peek() == r'}') {
            pos += 1
            return JObj(pairs)
        }
        while (true) {
            skipWs()
            let key = parseStr()
            skipWs()
            expect(r':')
            let v = parseValue()
            pairs.add((key, v))
            skipWs()
            if (pos >= rs.size) {
                throw SdkError("JSON 对象未闭合", "failed")
            }
            if (peek() == r',') {
                pos += 1
                continue
            }
            expect(r'}')
            return JObj(pairs)
        }
        throw SdkError("unreachable", "failed")
    }

    func parseArr(): JsonValue {
        expect(r'[')
        let items = ArrayList<JsonValue>()
        skipWs()
        if (pos < rs.size && peek() == r']') {
            pos += 1
            return JArr(items)
        }
        while (true) {
            let v = parseValue()
            items.add(v)
            skipWs()
            if (pos >= rs.size) {
                throw SdkError("JSON 数组未闭合", "failed")
            }
            if (peek() == r',') {
                pos += 1
                continue
            }
            expect(r']')
            return JArr(items)
        }
        throw SdkError("unreachable", "failed")
    }

    func parseStr(): String {
        expect(r'"')
        let sb = StringBuilder()
        while (true) {
            if (pos >= rs.size) {
                throw SdkError("JSON 字符串未闭合", "failed")
            }
            let r = rs[pos]
            if (r == r'"') {
                pos += 1
                return sb.toString()
            }
            if (r == r'\\') {
                pos += 1
                if (pos >= rs.size) {
                    throw SdkError("JSON 转义意外结束", "failed")
                }
                let e = rs[pos]
                pos += 1
                match (e) {
                    case r'"' => sb.append("\"")
                    case r'\\' => sb.append("\\")
                    case r'/' => sb.append("/")
                    case r'n' => sb.append("\n")
                    case r'r' => sb.append("\r")
                    case r't' => sb.append("\t")
                    case r'b' => sb.append("\b")
                    case r'f' => sb.append("\f")
                    case r'u' => parseUnicodeEscape(sb)
                    case _ => throw SdkError("JSON 非法转义", "failed")
                }
                continue
            }
            sb.append(r.toString())
            pos += 1
        }
        throw SdkError("unreachable", "failed")
    }

    func parseHex4(): UInt32 {
        var v: UInt32 = 0
        for (i in 0..4) {
            if (pos >= rs.size) {
                throw SdkError("JSON \\u 转义意外结束", "failed")
            }
            let r = rs[pos]
            pos += 1
            let d: UInt32 = match (r) {
                case r'0' => 0
                case r'1' => 1
                case r'2' => 2
                case r'3' => 3
                case r'4' => 4
                case r'5' => 5
                case r'6' => 6
                case r'7' => 7
                case r'8' => 8
                case r'9' => 9
                case r'a' => 10
                case r'b' => 11
                case r'c' => 12
                case r'd' => 13
                case r'e' => 14
                case r'f' => 15
                case r'A' => 10
                case r'B' => 11
                case r'C' => 12
                case r'D' => 13
                case r'E' => 14
                case r'F' => 15
                case _ => throw SdkError("JSON 非法十六进制", "failed")
            }
            v = (v << 4) | d
        }
        return v
    }

    func parseUnicodeEscape(sb: StringBuilder): Unit {
        let u = parseHex4()
        var code = u
        // 代理对合并
        if (u >= 0xD800 && u <= 0xDBFF && pos + 1 < rs.size && rs[pos] == r'\\' && rs[pos + 1] == r'u') {
            pos += 2
            let lo = parseHex4()
            if (lo >= 0xDC00 && lo <= 0xDFFF) {
                code = 0x10000 + ((u - 0xD800) << 10) + (lo - 0xDC00)
            } else {
                sb.append(Rune(u).toString())
                code = lo
            }
        }
        sb.append(Rune(code).toString())
    }

    func parseNum(): JsonValue {
        let start = pos
        if (pos < rs.size && (peek() == r'-' || peek() == r'+')) {
            pos += 1
        }
        while (pos < rs.size) {
            let r = peek()
            if (r == r'0' || r == r'1' || r == r'2' || r == r'3' || r == r'4'
                || r == r'5' || r == r'6' || r == r'7' || r == r'8' || r == r'9'
                || r == r'.' || r == r'e' || r == r'E' || r == r'-' || r == r'+') {
                pos += 1
            } else {
                break
            }
        }
        if (pos == start) {
            throw SdkError("JSON 非法数字", "failed")
        }
        let textSb = StringBuilder()
        for (r in rs[start..pos]) {
            textSb.append(r.toString())
        }
        let text = textSb.toString()
        try {
            let n = Float64.parse(text)
            return JNum(n)
        } catch (e: Exception) {
            throw SdkError("JSON 非法数字: ${text}", "failed")
        }
    }
}
