// guest-protocol-1 帧编解码与应答语义（对齐 SDK/ability-sdk-ts lib/protocol.js）。
package leaf_ability_sdk

public const PROTOCOL_VERSION: String = "1"

// 框架版本 schema 段（`cangjie@1` → `1`，`cj@1` 同义）。
public func schemaVersion(frameworkVersion: String): String {
    let at = findRune(frameworkVersion, r'@')
    if (at >= 0) {
        return frameworkVersion[at + 1..frameworkVersion.size]
    }
    return frameworkVersion
}

func findRune(s: String, target: Rune): Int64 {
    var i: Int64 = 0
    for (r in s.runes()) {
        if (r == target) {
            return i
        }
        i += 1
    }
    return -1
}

var frameIdSeq: Int64 = 0

public func nextFrameId(): String {
    frameIdSeq += 1
    return "g${frameIdSeq}"
}

// 帧：{"id":"…","reply_to"?:"…","body":{...}}
public func encodeFrame(frame: JsonValue): String {
    return frame.toJsonString() + "\n"
}

public func decodeFrame(line: String): JsonValue {
    let trimmed = trimLine(line)
    if (trimmed == "") {
        throw SdkError("空帧", "failed")
    }
    return jsonParse(trimmed)
}

public func frameReply(to: String, body: JsonValue): JsonValue {
    return jObjOf([
        ("id", JStr(nextFrameId())),
        ("reply_to", JStr(to)),
        ("body", body),
    ])
}

public func frameRequest(body: JsonValue): JsonValue {
    return jObjOf([
        ("id", JStr(nextFrameId())),
        ("body", body),
    ])
}

// ok → value（缺省 null）；err → 抛 SdkError(kind)。
public func okValue(frame: JsonValue): JsonValue {
    let body = jObjGet(frame, "body")
    let btype = jAsStr(jObjGet(body, "type"))
    if (btype == Some("ok")) {
        let v = jObjGet(body, "value")
        return v
    }
    if (btype == Some("err")) {
        let e = jObjGet(body, "error")
        let msg = match (jAsStr(jObjGet(e, "message"))) {
            case Some(m) => m
            case None => "unknown"
        }
        let kind = match (jAsStr(jObjGet(e, "kind"))) {
            case Some(k) => k
            case None => "failed"
        }
        throw SdkError(msg, kind)
    }
    throw SdkError("远程应答类型不符: ${body.toJsonString()}", "failed")
}

func trimLine(line: String): String {
    return line.trimAscii()
}

public class SdkError <: Exception {
    public let kind: String

    public init(message: String, kind: String) {
        super(message)
        this.kind = kind
    }
}

public func failSdk(message: String, kind: String): SdkError {
    return SdkError(message, kind)
}

func isWs(r: Rune): Bool {
    return r == r' ' || r == r'\t' || r == r'\n' || r == r'\r'
}
