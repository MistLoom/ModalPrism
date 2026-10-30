// serve 入口：从 ability.yml + handler 表启动 guest（对齐 serveAbility / serve_ability）。
package leaf_ability_sdk

import std.collection.ArrayList
import std.collection.HashMap
import std.console.*
import std.env.*
import std.fs.*
import std.io.*
import std.sync.*

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
        // 与 Node/Python 对齐：自动包装长任务轮询
        spec.on(m[0], desc, jObjOf([("type", JStr("object"))]), withLongTask(h))
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



// ---------------------------------------------------------------------------
// 长任务（协议级）：pending + {handle} 轮询，对齐 ability-sdk-ts / -py。
// start 即 spawn 后台协程执行（真后台，主循环可继续泵帧）；poll 只读状态。
// ---------------------------------------------------------------------------

internal class LongTaskSlot {
    var state: String = "running" // running | done | failed
    var data: JsonValue = JNull
    var error: String = ""
    var work: ?(() -> JsonValue) = None
}

// serve 主循环单线程：pending 通过包级槽位把闭包交给 withLongTask（无法塞进 JsonValue）。
var gPendingWork: ?(() -> JsonValue) = None

/**
 * 标记长任务。handler 应 `return pending({ => ... })`；
 * 立即返回 `{task_handle}`，调用方以同一方法 + `{handle}` 轮询。
 */
public func pending(work: () -> JsonValue): JsonValue {
    gPendingWork = Some(work)
    return jObjOf([("$pending", JBool(true))])
}

internal class LongTaskBox {
    let tasks: HashMap<String, LongTaskSlot>
    let mutex: ReentrantMutex
    var seq: Int64

    init() {
        this.tasks = HashMap<String, LongTaskSlot>()
        this.mutex = ReentrantMutex()
        this.seq = 0
    }

    func nextHandle(): String {
        mutex.lock()
        this.seq = this.seq + 1
        let h = "task-${this.seq}"
        mutex.unlock()
        return h
    }

    func runSlot(slot: LongTaskSlot): Unit {
        mutex.lock()
        let w = slot.work
        slot.work = None
        mutex.unlock()
        match (w) {
            case None => return
            case Some(work) =>
                try {
                    let v = work()
                    mutex.lock()
                    slot.data = v
                    slot.state = "done"
                    mutex.unlock()
                } catch (e: Exception) {
                    mutex.lock()
                    slot.error = "${e.message}"
                    slot.state = "failed"
                    mutex.unlock()
                }
        }
    }

    func poll(handle: String): JsonValue {
        mutex.lock()
        let slot = tasks.get(handle)
        var reply: JsonValue
        match (slot) {
            case None =>
                reply = jObjOf([
                    ("state", JStr("not_found")),
                    ("handle", JStr(handle)),
                ])
            case Some(s) =>
                if (s.state == "running") {
                    reply = jObjOf([
                        ("state", JStr("running")),
                        ("handle", JStr(handle)),
                    ])
                } else {
                    this.tasks.remove(handle)
                    if (s.state == "done") {
                        reply = jObjOf([
                            ("state", JStr("done")),
                            ("data", s.data),
                        ])
                    } else {
                        reply = jObjOf([
                            ("state", JStr("failed")),
                            ("error", JStr(s.error)),
                        ])
                    }
                }
        }
        mutex.unlock()
        return reply
    }

    func start(work: () -> JsonValue): JsonValue {
        let handle = nextHandle()
        let slot = LongTaskSlot()
        slot.work = Some(work)
        mutex.lock()
        this.tasks.add(handle, slot)
        mutex.unlock()
        // 真后台：立即在后台协程执行，不占主循环。
        spawn {
            runSlot(slot)
        }
        return jObjOf([("task_handle", JStr(handle))])
    }
}

/** 包装业务 handler：拦截 args.handle 轮询；识别 pending(...) 返回值。 */
public func withLongTask(fn: Handler): Handler {
    let box = LongTaskBox()
    return { ctx, args =>
        if (let Some(h) <- jAsStr(jObjGet(args, "handle"))) {
            return box.poll(h)
        }
        let result = fn(ctx, args)
        if (let Some(flag) <- jAsBool(jObjGet(result, "$pending"))) {
            if (flag) {
                let work = gPendingWork
                gPendingWork = None
                match (work) {
                    case Some(w) => return box.start(w)
                    case None => throw SdkError("pending() 未登记工作闭包", "failed")
                }
            }
        }
        return result
    }
}


func resolveAbilityRoot(explicit: ?String): String {
    if (let Some(r) <- explicit) {
        return r
    }
    // 与 Node/Python 同口径：LEAF_ABILITY_ROOT 优先于 cwd 上溯。
    if (let Some(envRoot) <- getVariable("LEAF_ABILITY_ROOT")) {
        if (envRoot != "") {
            return envRoot
        }
    }
    // 未命中环境变量：从工作目录逐级向上找 ability.yml（相对路径默认以 cwd 为基）。
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
    // 宿主关闭 stdin（断开/退出）时 readln 抛 I/O/EOF 异常：视为退出指令，正常返回 None。
    try {
        let line = readln()
        return Some(line)
    } catch (e: Exception) {
        return None
    }
}

func stdioWrite(line: String): Unit {
    print(line)
    // 重定向 stdout 是缓冲写：不 flush 宿主在读端拿到空数据，握手会超时。
    Console.stdOut.flush()
}
