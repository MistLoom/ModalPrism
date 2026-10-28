// 能力包 `ability.yml` 声明子集解析（对齐 lib/yaml-lite.js，仅取 id/framework_version/
// stateful/methods/events/depends，无第三方依赖）。
package leaf_ability_sdk

import std.collection.ArrayList

public class AbilityDecl {
    public var id: String = ""
    public var frameworkVersion: String = "1"
    public var stateful: Bool = false
    public var methods: ArrayList<(String, String)> = ArrayList<(String, String)>() // (id, description)
    public var events: ArrayList<(String, String)> = ArrayList<(String, String)>()  // (name, dir)
    public var depends: ArrayList<(String, ?String)> = ArrayList<(String, ?String)>() // (id, alias)
}

public func loadAbilityYaml(text: String): AbilityDecl {
    let decl = AbilityDecl()
    let lines = splitLines(text)
    decl.id = matchScalar(lines, "id") ?? ""
    decl.frameworkVersion = matchScalar(lines, "framework_version") ?? "1"
    for (line in lines) {
        if (line.startsWith("stateful:") && line.contains("true")) {
            decl.stateful = true
        }
    }
    if (decl.id == "") {
        throw SdkError("ability.yml 缺少 id", "failed")
    }

    let methodsBody = matchSection(lines, "methods")
    if (let Some(body) <- methodsBody) {
        for (chunk in splitChunks(body, "- id:")) {
            let mid = matchKey(chunk, "id") ?? ""
            if (mid != "") {
                let desc = matchKey(chunk, "description") ?? ""
                decl.methods.add((mid, desc))
            }
        }
    }

    let eventsBody = matchSection(lines, "events")
    if (let Some(body) <- eventsBody) {
        for (chunk in splitChunks(body, "- name:")) {
            let name = matchKey(chunk, "name") ?? ""
            if (name != "") {
                let dir = matchKey(chunk, "dir") ?? "emit"
                decl.events.add((name, dir))
            }
        }
    }

    let dependsBody = matchSection(lines, "depends")
    if (let Some(body) <- dependsBody) {
        let trimmedBody = body.trimAscii()
        if (trimmedBody != "[]") {
            for (chunk in splitChunks(body, "- ")) {
                let id = matchKey(chunk, "id") ?? ""
                if (id != "") {
                    let alias = matchKey(chunk, "alias")
                    decl.depends.add((id, alias))
                    continue
                }
                // 简写：`- ability.id`
                let simple = chunk.trimAscii()
                if (simple != "" && !simple.contains(":")) {
                    decl.depends.add((simple, None))
                }
            }
        }
    }

    return decl
}

func splitLines(text: String): ArrayList<String> {
    let out = ArrayList<String>()
    let sb = StringBuilder()
    for (r in text.runes()) {
        if (r == r'\n') {
            out.add(sb.toString())
            sb.reset()
            continue
        }
        if (r == r'\r') {
            continue
        }
        sb.append(r.toString())
    }
    out.add(sb.toString())
    return out
}

// 顶层标量：`^key: value`
func matchScalar(lines: ArrayList<String>, key: String): ?String {
    let prefix = key + ":"
    for (line in lines) {
        if (line.startsWith(prefix)) {
            return unquote(line[prefix.size..line.size].trimAscii())
        }
    }
    return None
}

// key 下的缩进块（下一个非缩进的 `xxx:` 顶层键截止）。
func matchSection(lines: ArrayList<String>, key: String): ?String {
    let prefix = key + ":"
    var start: ?Int64 = None
    for (i in 0..lines.size) {
        if (lines[i].startsWith(prefix)) {
            start = Some(i + 1)
            break
        }
    }
    if (let Some(s) <- start) {
        let body = ArrayList<String>()
        for (i in s..lines.size) {
            let line = lines[i]
            if (isTopLevelKey(line)) {
                break
            }
            body.add(line)
        }
        return Some(joinLines(body))
    }
    return None
}

func isTopLevelKey(line: String): Bool {
    if (line == "" || line.startsWith(" ") || line.startsWith("\t") || line.startsWith("-")) {
        return false
    }
    for (r in line.runes()) {
        if (r == r':') {
            return true
        }
        if (!(r.isAsciiLetter() || (r >= r'0' && r <= r'9') || r == r'_' || r == r'.')) {
            return false
        }
    }
    return false
}

func joinLines(lines: ArrayList<String>): String {
    let sb = StringBuilder()
    for (i in 0..lines.size) {
        if (i > 0) {
            sb.append("\n")
        }
        sb.append(lines[i])
    }
    return sb.toString()
}

func splitChunks(body: String, marker: String): ArrayList<String> {
    let out = ArrayList<String>()
    let lines = splitLines(body)
    let current = ArrayList<String>()
    for (line in lines) {
        let t = line.trimAscii()
        if (t.startsWith("- ")) {
            // 新块开始：剥离 "- " 前缀，让首键以 `key:` 形式被 matchKey 命中
            if (current.size > 0) {
                out.add(joinLines(current))
                current.clear()
            }
            current.add(t[2..t.size])
            continue
        }
        if (current.size > 0 && t != "") {
            current.add(t)
        }
    }
    if (current.size > 0) {
        out.add(joinLines(current))
    }
    return out
}

func matchKey(text: String, key: String): ?String {
    let lines = splitLines(text)
    for (line in lines) {
        let t = line.trimAscii()
        let prefix = key + ":"
        if (t.startsWith(prefix)) {
            let raw = t[prefix.size..t.size].trimAscii()
            if (raw == "") {
                return None
            }
            return unquote(raw)
        }
    }
    return None
}

func unquote(s: String): String {
    if (s.size >= 2) {
        let rs = runeArray(s)
        let first = rs[0]
        let last = rs[rs.size - 1]
        if ((first == r'"' && last == r'"') || (first == r'\'' && last == r'\'')) {
            return s[1..s.size - 1]
        }
    }
    return s
}
