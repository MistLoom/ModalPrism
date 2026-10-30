// 测试桩：模拟宿主侧帧。脚本帧按序投递；ctx 请求按 op 名自动回罐头应答。
package leaf_ability_sdk

import std.collection.ArrayList
import std.collection.HashMap
import std.collection.HashSet

internal class FakeHost {
    var inLines: ArrayList<String>
    var out: ArrayList<JsonValue>
    var ctxReplies: HashMap<String, JsonValue>
    // subscribe 应答后要补投的事件帧（原文）
    var autoEvents: ArrayList<String>
    var replied: HashSet<String>
    var log: ArrayList<String>

    init() {
        this.inLines = ArrayList<String>()
        this.out = ArrayList<JsonValue>()
        this.ctxReplies = HashMap<String, JsonValue>()
        this.autoEvents = ArrayList<String>()
        this.replied = HashSet<String>()
        this.log = ArrayList<String>()
    }

    func input(): ?String {
        if (inLines.size > 0) {
            let l = inLines[0]
            inLines.remove(0..1)
            log.add("script:" + l)
            return Some(l)
        }
        // 自动应答最近的未应答 ctx 请求（从后往前扫，中间可能插入 run_handle 应答）
        var i = out.size - 1
        while (i >= 0) {
            let last = out[i]
            let body = jObjGet(last, "body")
            let rid = jAsStr(jObjGet(last, "id")) ?? ""
            if (jAsStr(jObjGet(body, "type")) == Some("ctx") && rid != "" && !replied.contains(rid)) {
                replied.add(rid)
                let opName = jAsStr(jObjGet(jObjGet(body, "op"), "op")) ?? ""
                let value = match (ctxReplies.get(opName)) {
                    case Some(v) => v
                    case None => JNull
                }
                let reply = frameReply(rid, jObjOf([
                    ("type", JStr("ok")),
                    ("value", value),
                ]))
                log.add("autoreply:" + rid + ":" + opName)
                return Some(encodeFrame(reply))
            }
            i -= 1
        }
        if (autoEvents.size > 0) {
            let l = autoEvents[0]
            autoEvents.remove(0..1)
            log.add("autoevent:" + l)
            return Some(l)
        }
        log.add("exhausted")
        return None
    }

    func write(line: String): Unit {
        out.add(decodeFrame(line))
    }

    func runServe(provider: AbilityProvider): Unit {
        serveWithIO(provider, input, write)
    }

    func push(line: String): Unit {
        inLines.add(line)
    }

    // 找到最近的 body.type == "ctx" 且 op == opName 的出站帧
    func findCtxOp(opName: String): ?JsonValue {
        var i = out.size - 1
        while (i >= 0) {
            let f = out[i]
            let body = jObjGet(f, "body")
            if (jAsStr(jObjGet(body, "type")) == Some("ctx")) {
                if (jAsStr(jObjGet(jObjGet(body, "op"), "op")) == Some(opName)) {
                    return Some(body)
                }
            }
            i -= 1
        }
        return None
    }

    func replyOf(cmdIndex: Int64): JsonValue {
        return jObjGet(out[cmdIndex], "body")
    }

    // 按 reply_to 找命令应答（ctx 请求会插入 out，索引会位移，优先用本方法）。
    func replyFor(cmdId: String): JsonValue {
        for (f in out) {
            if (jAsStr(jObjGet(f, "reply_to")) == Some(cmdId)) {
                return jObjGet(f, "body")
            }
        }
        return JNull
    }
}

// cmd 帧构造
internal func cmdFrame(id: String, body: JsonValue): String {
    return jObjOf([
        ("id", JStr(id)),
        ("body", body),
    ]).toJsonString()
}

internal func helloCmd(id: String): String {
    return cmdFrame(id, jObjOf([("type", JStr("hello"))]))
}

internal func createCmd(id: String): String {
    return cmdFrame(id, jObjOf([
        ("type", JStr("create")),
        ("config", JNull),
    ]))
}

internal func invokeCmd(id: String, method: String, args: JsonValue): String {
    return cmdFrame(id, jObjOf([
        ("type", JStr("invoke")),
        ("instance", JStr("inst-1")),
        ("method", JStr(method)),
        ("chain_id", JStr("chain-9")),
        ("span", JNum(3.0)),
        ("args", args),
    ]))
}

internal func lifecycleCmd(id: String, phase: String): String {
    return cmdFrame(id, jObjOf([
        ("type", JStr("lifecycle")),
        ("instance", JStr("inst-1")),
        ("phase", JStr(phase)),
        ("chain_id", JStr("chain-9")),
        ("span", JNum(3.0)),
    ]))
}

internal func okBody(v: JsonValue): JsonValue {
    return jObjOf([
        ("type", JStr("ok")),
        ("value", v),
    ])
}
