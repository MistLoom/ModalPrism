// serve 入口：从 ability.yml + handler 表启动 guest（对齐 serveAbility / serve_ability）。
package leaf_ability_sdk

import std.collection.ArrayList
import std.collection.HashMap
import std.fs.*
import std.io.*

/**
 * 业务用法：
 *
 *   let handlers = HashMap<String, Handler>()
 *   handlers.add("run", { (ctx, args) => ... })
 *   serveAbility(handlers)
 *
 * ability.yml 定位顺序：显式 root → 环境变量 LEAF_ABILITY_ROOT → 从工作目录逐级向上。
 */
public func serveAbility(handlers: HashMap<String, Handler>): Unit {
    serveAbility(handlers, HashMap<String, (GuestCtx) -> Unit>(), None)
}

public func serveAbility(handlers: HashMap<String, Handler>,
    lifecycle: HashMap<String, (GuestCtx) -> Unit>): Unit {
    serveAbility(handlers, lifecycle, None)
}

public func serveAbility(handlers: HashMap<String, Handler>,
    lifecycle: HashMap<String, (GuestCtx) -> Unit>, root: ?String): Unit {
    let actualRoot = resolveAbilityRoot(root)
    let text = readTextFile(actualRoot + "/ability.yml")
    let decl = loadAbilityYaml(text)
    // framework_version：`cangjie@1`（`cj@1` 同义），与 SDK 的 schema 版本必须一致。
    let sv = schemaVersion(decl.frameworkVersion)
    if (sv != "1") {
        throw SdkError("ability.yml framework_version 与 SDK 不匹配", "failed")
    }
    let provider = buildProvider(decl, handlers, lifecycle)
    serveWithIO(provider, stdioInput, stdioWrite)
}

public func buildProvider(decl: AbilityDecl, handlers: HashMap<String, Handler>,
    lifecycle: HashMap<String, (GuestCtx) -> Unit>): AbilityProvider {
    let spec = AbilitySpec.must(decl.id)
    spec.stateful = decl.stateful
    for (d in decl.depends) {
        if (let Some(alias) <- d[1]) {
            spec.dependsAlias(d[0], alias)
        }
    }
    for (ev in decl.events) {
        if (ev[1] == "emit") {
            spec.eventEmit(ev[0], "", jObjOf([("type", JStr("object"))]))
        }
    }
    for (m in decl.methods) {
        if (!handlers.contains(m[0])) {
            throw SdkError("缺少方法处理器: ${m[0]}", "failed")
        }
        let h = match (handlers.get(m[0])) {
            case Some(fn) => fn
            case None => throw SdkError("缺少方法处理器: ${m[0]}", "failed")
        }
        let desc = if (m[1] == "") { m[0] } else { m[1] }
        spec.on(m[0], desc, jObjOf([("type", JStr("object"))]), h)
    }
    let base = spec.intoProvider()
    return LifecycleProvider(base, lifecycle)
}

internal class LifecycleProvider <: AbilityProvider {
    let base: AbilityProvider
    let lifecycle: HashMap<String, (GuestCtx) -> Unit>

    init(base: AbilityProvider, lifecycle: HashMap<String, (GuestCtx) -> Unit>) {
        this.base = base
        this.lifecycle = lifecycle
    }

    public func manifest(): JsonValue {
        return base.manifest()
    }

    public func create(config: JsonValue): AbilityInstance {
        return LifecycleInstance(base.create(config), lifecycle)
    }
}

internal class LifecycleInstance <: AbilityInstance {
    let base: AbilityInstance
    let lifecycle: HashMap<String, (GuestCtx) -> Unit>

    init(base: AbilityInstance, lifecycle: HashMap<String, (GuestCtx) -> Unit>) {
        this.base = base
        this.lifecycle = lifecycle
    }

    public func manifest(): JsonValue {
        return base.manifest()
    }

    public func onLifecycle(phase: String, ctx: GuestCtx): Unit {
        if (let Some(hook) <- lifecycle.get(phase)) {
            hook(ctx)
        }
        base.onLifecycle(phase, ctx)
    }

    public func invoke(method: String, ctx: GuestCtx, args: JsonValue): JsonValue {
        return base.invoke(method, ctx, args)
    }
}

func resolveAbilityRoot(explicit: ?String): String {
    if (let Some(r) <- explicit) {
        return r
    }
    // 本工具链无环境变量 API：从工作目录逐级向上找 ability.yml（相对路径默认以 cwd 为基）。
    var up = 0
    while (up <= 6) {
        var dir = "."
        for (_i in 0..up) {
            dir = dir + "/.."
        }
        if (fileExists(dir + "/ability.yml")) {
            return dir
        }
        up += 1
    }
    return "."
}

func fileExists(path: String): Bool {
    try {
        let f = File(path, OpenMode.Read)
        f.close()
        return true
    } catch (e: Exception) {
        return false
    }
}

public func readTextFile(path: String): String {
    let f = File(path, OpenMode.Read)
    let sb = StringBuilder()
    let buf = Array<UInt8>(4096, repeat: UInt8(0))
    while (true) {
        let n = f.read(buf)
        if (n <= 0) {
            break
        }
        sb.append(String.fromUtf8(buf[0..n].toArray()))
    }
    f.close()
    return sb.toString()
}

func stdioInput(): ?String {
    let line = readln()
    return Some(line)
}

func stdioWrite(line: String): Unit {
    print(line)
}
