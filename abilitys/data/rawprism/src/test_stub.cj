// 内部测试桩：以最小宿主侧帧脚本驱动 serveWithIO，检验 Guest 线协议与方法结果。
package rawprism_guest

import leaf_ability_sdk.*
import std.collection.*

// 最小宿主：按序投递脚本帧，捕获客端出站帧。
class GuestFixture {
    var inLines: ArrayList<String>
    var out: ArrayList<String>

    init() {
        this.inLines = ArrayList<String>()
        this.out = ArrayList<String>()
    }

    func input(): ?String {
        if (inLines.size > 0) {
            let l = inLines[0]
            inLines.remove(0..1)
            return Some(l)
        }
        return None
    }

    func write(line: String): Unit {
        out.add(line)
    }

    func push(line: String): Unit {
        inLines.add(line)
    }

    // 按 reply_to 找命令应答的 value。
    func replyValue(cmdId: String): JsonValue {
        for (line in out) {
            let f = decodeFrame(line)
            if (jAsStr(jObjGet(f, "reply_to")) == Some(cmdId)) {
                let body = jObjGet(f, "body")
                if (jAsStr(jObjGet(body, "type")) == Some("ok")) {
                    return jObjGet(body, "value")
                }
                return body
            }
        }
        return JNull
    }
}

// 从工作目录逐级向上定位 ability.yml（与 SDK 的 resolveAbilityRoot 同思路）。
func findAbilityYml(): String {
    var up = 0
    while (up <= 8) {
        var dir = "."
        for (_i in 0..up) {
            dir = dir + "/.."
        }
        if (fileReadable(dir + "/ability.yml")) {
            return dir + "/ability.yml"
        }
        up += 1
    }
    return "ability.yml"
}

func fileReadable(path: String): Bool {
    try {
        let t = readTextFile(path)
        return t.size > 0
    } catch (e: Exception) {
        return false
    }
}

func guestProvider(): AbilityProvider {
    let decl = loadAbilityYaml(readTextFile(findAbilityYml()))
    return buildProvider(decl, rawprismHandlers(), HashMap<String, (GuestCtx) -> Unit>())
}

// 一次性会话：hello → create → invoke(method, args)，返回 invoke 的 value。
func guestInvoke(method: String, args: JsonValue): JsonValue {
    let fx = GuestFixture()
    fx.push(jObjOf([
        ("id", JStr("h1")),
        ("body", jObjOf([("type", JStr("hello"))])),
    ]).toJsonString())
    fx.push(jObjOf([
        ("id", JStr("c1")),
        ("body", jObjOf([("type", JStr("create")), ("config", JNull)])),
    ]).toJsonString())
    fx.push(jObjOf([
        ("id", JStr("i1")),
        ("body", jObjOf([
            ("type", JStr("invoke")),
            ("instance", JStr("inst-1")),
            ("method", JStr(method)),
            ("chain_id", JStr("chain-1")),
            ("span", JNum(1.0)),
            ("args", args),
        ])),
    ]).toJsonString())
    serveWithIO(guestProvider(), fx.input, fx.write)
    return fx.replyValue("i1")
}

// manifest（hello 应答）。
func guestManifest(): JsonValue {
    let fx = GuestFixture()
    fx.push(jObjOf([
        ("id", JStr("h1")),
        ("body", jObjOf([("type", JStr("hello"))])),
    ]).toJsonString())
    serveWithIO(guestProvider(), fx.input, fx.write)
    let f = decodeFrame(fx.out[0])
    return jObjGet(f, "body")
}

// 方便断言的 JSON 文本化。
func js(v: JsonValue): String {
    v.toJsonString()
}
