// Guest 桥与 serve 主循环测试：用 FakeHost 模拟宿主侧帧，覆盖能力矩阵。
package leaf_ability_sdk

import std.collection.ArrayList
import std.collection.HashMap
import std.unittest.*
import std.unittest.testmacro.*

// 记录 lifecycle 钩子与 undo 执行
internal class Recorder {
    var phases: ArrayList<String>
    var undos: ArrayList<String>

    init() {
        this.phases = ArrayList<String>()
        this.undos = ArrayList<String>()
    }
}

internal class SimpleUndo <: Undo {
    let recorder: Recorder
    let name: String

    init(recorder: Recorder, name: String) {
        this.recorder = recorder
        this.name = name
    }

    public func run(): Unit {
        recorder.undos.add(name)
    }
}

internal class SimpleEffect <: Effect {
    let recorder: Recorder
    let name: String

    init(recorder: Recorder, name: String) {
        this.recorder = recorder
        this.name = name
    }

    public func apply(_ctx: GuestCtx): Undo {
        return SimpleUndo(recorder, name)
    }
}

internal func recHandler(rec: Recorder, step: (GuestCtx, JsonValue) -> JsonValue): Handler {
    return { ctx, args => step(ctx, args) }
}

internal func newHost(): FakeHost {
    let host = FakeHost()
    host.ctxReplies.add("call", JNum(7.0))
    host.ctxReplies.add("dep", JStr("dep-inst-1"))
    host.ctxReplies.add("local_get", jObjOf([
        ("hit", JBool(true)),
        ("value", JNum(42.0)),
    ]))
    host.ctxReplies.add("local_set", JNull)
    host.ctxReplies.add("emit", JNull)
    host.ctxReplies.add("catalog", jArrOf([JStr("a.x"), JStr("a.y")]))
    host.ctxReplies.add("chain_status", jObjOf([("span", JNum(3.0))]))
    host.ctxReplies.add("load", jObjOf([("instance", JStr("dep-inst-1"))]))
    host.ctxReplies.add("unload", JNull)
    host.ctxReplies.add("subscribe", jObjOf([("sub", JStr("s1"))]))
    host.ctxReplies.add("handle", jObjOf([("id", JNum(3.0))]))
    host.ctxReplies.add("unhandle", JNull)
    host.ctxReplies.add("feedback", JNull)
    host.ctxReplies.add("share", jObjOf([("ctx_id", JStr("ctx-2"))]))
    host.ctxReplies.add("event_next", jObjOf([("passed", JBool(true))]))
    return host
}

internal func specProvider(h: Handler, rec: Recorder): AbilityProvider {
    let spec = AbilitySpec.must("algo.echo")
    spec.on("run", "回显", jObjOf([("type", JStr("object"))]), h)
    let base = spec.intoProvider()
    let hooks = HashMap<String, (GuestCtx) -> Unit>()
    hooks.add("start", { ctx => rec.phases.add("start:" + (ctx.instance ?? "?")) })
    return buildProviderFromSpec(base, hooks)
}

// 直接把 lifecycle 钩子包在 provider 外层（不经 ability.yml）。
internal func buildProviderFromSpec(base: AbilityProvider,
    hooks: HashMap<String, (GuestCtx) -> Unit>): AbilityProvider {
    return HookedProvider(base, hooks)
}

internal class HookedProvider <: AbilityProvider {
    let base: AbilityProvider
    let hooks: HashMap<String, (GuestCtx) -> Unit>

    init(base: AbilityProvider, hooks: HashMap<String, (GuestCtx) -> Unit>) {
        this.base = base
        this.hooks = hooks
    }

    public func manifest(): JsonValue {
        return base.manifest()
    }

    public func create(config: JsonValue): AbilityInstance {
        return HookedInstance(base.create(config), hooks)
    }
}

internal class HookedInstance <: AbilityInstance {
    let base: AbilityInstance
    let hooks: HashMap<String, (GuestCtx) -> Unit>

    init(base: AbilityInstance, hooks: HashMap<String, (GuestCtx) -> Unit>) {
        this.base = base
        this.hooks = hooks
    }

    public func manifest(): JsonValue {
        return base.manifest()
    }

    public func onLifecycle(phase: String, ctx: GuestCtx): Unit {
        if (let Some(hook) <- hooks.get(phase)) {
            hook(ctx)
        }
        base.onLifecycle(phase, ctx)
    }

    public func invoke(method: String, ctx: GuestCtx, args: JsonValue): JsonValue {
        return base.invoke(method, ctx, args)
    }
}

internal func hasCtxOp(host: FakeHost, opName: String): Bool {
    return match (host.findCtxOp(opName)) {
        case Some(_) => true
        case None => false
    }
}

// 解包 findCtxOp 结果，未找到时返回 JNull（测试内先 hasCtxOp 断言存在）。
internal func ctxOpOf(host: FakeHost, opName: String): JsonValue {
    return match (host.findCtxOp(opName)) {
        case Some(b) => b
        case None => JNull
    }
}

// 取第 i 个出站帧的 error.message 文本，缺失时返回空串。
internal func msgOf(host: FakeHost, i: Int64): String {
    let body = host.replyOf(i)
    return match (jAsStr(jObjGet(jObjGet(body, "error"), "message"))) {
        case Some(m) => m
        case None => ""
    }
}

@Test
public func testHandshakeManifest(): Unit {
    let host = newHost()
    host.push(helloCmd("c1"))
    host.push(createCmd("c2"))
    host.push(cmdFrame("c3", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ _ctx, args => args }, Recorder()))
    // hello → manifest 应答
    let m = host.replyOf(0)
    @Assert(jAsStr(jObjGet(m, "type")) == Some("manifest"))
    @Assert(jAsStr(jObjGet(host.out[0], "reply_to")) == Some("c1"))
    let manifest = jObjGet(m, "manifest")
    @Assert(jAsStr(jObjGet(manifest, "id")) == Some("algo.echo"))
    // create → ok
    @Assert(jAsStr(jObjGet(host.replyOf(1), "type")) == Some("ok"))
}

@Test
public func testInvokeBeforeCreateFails(): Unit {
    let host = newHost()
    host.push(invokeCmd("c1", "run", JNull))
    host.push(cmdFrame("c2", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ _ctx, args => args }, Recorder()))
    let body = host.replyOf(0)
    @Assert(jAsStr(jObjGet(body, "type")) == Some("err"))
    let error = jObjGet(body, "error")
    @Assert(jAsStr(jObjGet(error, "kind")) == Some("failed"))
    @Assert(jAsStr(jObjGet(error, "message")) == Some("尚未 Create"))
}

@Test
public func testInvokeUnknownMethod(): Unit {
    let host = newHost()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(invokeCmd("i", "nope", JNull))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ _ctx, args => args }, Recorder()))
    let body = host.replyFor("i")
    @Assert(jAsStr(jObjGet(body, "type")) == Some("err"))
    @Assert(jAsStr(jObjGet(jObjGet(body, "error"), "kind")) == Some("not_found"))
}

@Test
public func testInvokeEchoArgs(): Unit {
    let host = newHost()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    let args = jObjOf([("text", JStr("你好"))])
    host.push(invokeCmd("i", "run", args))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ _ctx, a => a }, Recorder()))
    let body = host.replyFor("i")
    @Assert(jAsStr(jObjGet(body, "type")) == Some("ok"))
    @Assert(jAsStr(jObjGet(jObjGet(body, "value"), "text")) == Some("你好"))
}

@Test
public func testLifecyclePhases(): Unit {
    let host = newHost()
    let rec = Recorder()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(lifecycleCmd("l1", "start"))
    host.push(lifecycleCmd("l2", "stop"))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ _ctx, args => args }, rec))
    @Assert(jAsStr(jObjGet(host.replyFor("l1"), "type")) == Some("ok"))
    @Assert(jAsStr(jObjGet(host.replyFor("l2"), "type")) == Some("ok"))
    @Assert(rec.phases.size == 1)
    @Assert(rec.phases[0] == "start:inst-1")
}

@Test
public func testCtxCall(): Unit {
    let host = newHost()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(invokeCmd("i", "run", JNull))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ ctx, _a => ctx.call("dep1", "run", jObjOf([("n", JNum(1.0))])) }, Recorder()))
    let body = host.replyFor("i")
    @Assert(jAsStr(jObjGet(body, "type")) == Some("ok"))
    @Assert(jAsNum(jObjGet(body, "value")) == Some(7.0))
    let opFrame = ctxOpOf(host, "call")
    @Assert(jAsStr(jObjGet(jObjGet(opFrame, "op"), "id_or_alias")) == Some("dep1"))
    @Assert(jAsStr(jObjGet(jObjGet(opFrame, "op"), "method")) == Some("run"))
    @Assert(jAsStr(jObjGet(opFrame, "ctx_id")) == Some("i"))
}

@Test
public func testCtxDepAndCatalog(): Unit {
    let host = newHost()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(invokeCmd("i", "run", JNull))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ ctx, _a =>
        let d = ctx.dep("dep1")
        let cat = ctx.catalog()
        return jObjOf([("dep", JStr(d)), ("cat", cat)])
    }, Recorder()))
    let body = host.replyFor("i")
    @Assert(jAsStr(jObjGet(body, "type")) == Some("ok"))
    @Assert(jAsStr(jObjGet(jObjGet(body, "value"), "dep")) == Some("dep-inst-1"))
    let cat = match (jAsArr(jObjGet(jObjGet(body, "value"), "cat"))) {
        case Some(l) => l
        case None => @Assert(false); ArrayList<JsonValue>()
    }
    @Assert(cat.size == 2)
}

@Test
public func testCtxLocalGetSet(): Unit {
    let host = newHost()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(invokeCmd("i", "run", JNull))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ ctx, _a =>
        ctx.localSet("k", JNum(42.0))
        let hit = ctx.localGet("k")
        let miss = ctx.localGet("nope")
        match (hit) {
            case Some(v) =>
                // 宿主桩 local_get 未命中时返回 hit=false
                match (miss) {
                    case Some(_) => return jObjOf([("v", v), ("miss", JBool(true))])
                    case None => return jObjOf([("v", v), ("miss", JBool(false))])
                }
            case None => return jObjOf([("v", JNull)])
        }
    }, Recorder()))
    let body = host.replyFor("i")
    @Assert(jAsStr(jObjGet(body, "type")) == Some("ok"))
    @Assert(jAsNum(jObjGet(jObjGet(body, "value"), "v")) == Some(42.0))
}

@Test
public func testCtxEmit(): Unit {
    let host = newHost()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(invokeCmd("i", "run", JNull))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ ctx, _a =>
        ctx.emit("pong", jObjOf([("t", JStr("x"))]))
        return okBody(JBool(true))
    }, Recorder()))
    let opFrame = ctxOpOf(host, "emit")
    @Assert(jAsStr(jObjGet(jObjGet(opFrame, "op"), "name")) == Some("pong"))
    @Assert(jAsStr(jObjGet(jObjGet(jObjGet(opFrame, "op"), "payload"), "t")) == Some("x"))
}

@Test
public func testCtxChainStatusAndLoadUnload(): Unit {
    let host = newHost()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(invokeCmd("i", "run", JNull))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ ctx, _a =>
        let cs = ctx.chainStatus(None)
        let inst = ctx.load(jObjOf([("id", JStr("dep1"))]))
        ctx.unload(inst)
        return jObjOf([("inst", JStr(inst))])
    }, Recorder()))
    let body = host.replyFor("i")
    @Assert(jAsStr(jObjGet(body, "type")) == Some("ok"))
    @Assert(jAsStr(jObjGet(jObjGet(body, "value"), "inst")) == Some("dep-inst-1"))
    @Assert(hasCtxOp(host, "chain_status"))
    @Assert(hasCtxOp(host, "load"))
    @Assert(hasCtxOp(host, "unload"))
}

@Test
public func testCtxSubscribeNext(): Unit {
    let host = newHost()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(invokeCmd("i", "run", JNull))
    // subscribe 应答后补投一条事件帧
    host.autoEvents.add(cmdFrame("e1", jObjOf([
        ("type", JStr("event")),
        ("sub", JStr("s1")),
        ("event", jObjOf([("n", JNum(1.0))])),
    ])))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ ctx, _a =>
        let sub = ctx.subscribe(jObjOf([("name", JStr("ping"))]))
        let ev = sub.next()
        return ev
    }, Recorder()))
    let body = host.replyFor("i")
    @Assert(jAsStr(jObjGet(body, "type")) == Some("ok"))
    @Assert(jAsNum(jObjGet(body, "value"), ) == Some(1.0) || jAsNum(jObjGet(jObjGet(body, "value"), "n")) == Some(1.0))
    let opFrame = ctxOpOf(host, "subscribe")
    @Assert(jAsStr(jObjGet(jObjGet(jObjGet(opFrame, "op"), "filter"), "name")) == Some("ping"))
}

@Test
public func testCtxHandleMessage(): Unit {
    let host = newHost()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(invokeCmd("i", "run", JNull))
    host.push(cmdFrame("rh", jObjOf([
        ("type", JStr("run_handle")),
        ("guest_id", JStr("h1")),
        ("event", jObjOf([("n", JNum(5.0))])),
    ])))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    let seen = Recorder()
    host.runServe(specProvider({ ctx, _a =>
        let reg = ctx.handle(jObjOf([("name", JStr("ping"))]),
            HandleHandler.message({ ev => seen.phases.add(ev.toJsonString()) }), false)
        @Assert(true)
        // 注册成功后不注销，让 run_handle 命中
        return JStr("registered")
    }, Recorder()))
    // invoke 应答
    @Assert(jAsStr(jObjGet(host.replyFor("i"), "type")) == Some("ok"))
    // run_handle 应答：onEvent 收到事件且原样返回
    let rhBody = host.replyFor("rh")
    @Assert(jAsStr(jObjGet(rhBody, "type")) == Some("ok"))
    @Assert(jAsNum(jObjGet(jObjGet(rhBody, "value"), "n")) == Some(5.0))
    @Assert(seen.phases.size == 1)
    let opFrame = ctxOpOf(host, "handle")
    @Assert(jAsStr(jObjGet(jObjGet(opFrame, "op"), "mode")) == Some("message"))
    @Assert(jAsStr(jObjGet(jObjGet(opFrame, "op"), "guest_id")) == Some("h1"))
}

@Test
public func testCtxHandleMiddlewareEventNext(): Unit {
    let host = newHost()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(invokeCmd("i", "run", JNull))
    host.push(cmdFrame("rh", jObjOf([
        ("type", JStr("run_handle")),
        ("guest_id", JStr("h1")),
        ("next_token", JStr("tok-1")),
        ("event", jObjOf([("n", JNum(5.0))])),
    ])))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ ctx, _a =>
        let _reg = ctx.handle(jObjOf([("name", JStr("ping"))]),
            HandleHandler.middleware({ ev, next => next(ev) }), false)
        return JStr("registered")
    }, Recorder()))
    let rhBody = host.replyFor("rh")
    @Assert(jAsStr(jObjGet(rhBody, "type")) == Some("ok"))
    // middleware next → event_next 应答值
    @Assert(jAsBool(jObjGet(jObjGet(rhBody, "value"), "passed")) == Some(true))
    @Assert(hasCtxOp(host, "event_next"))
}

@Test
public func testPushEffectUndoOnSuccess(): Unit {
    let host = newHost()
    let rec = Recorder()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(invokeCmd("i", "run", JNull))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ ctx, _a =>
        ctx.pushEffect(SimpleEffect(rec, "undo-1"))
        ctx.pushEffect(SimpleEffect(rec, "undo-2"))
        return JStr("done")
    }, rec))
    let body = host.replyFor("i")
    @Assert(jAsStr(jObjGet(body, "type")) == Some("ok"))
    // undo 倒序执行
    @Assert(rec.undos.size == 2)
    @Assert(rec.undos[0] == "undo-2")
    @Assert(rec.undos[1] == "undo-1")
}

@Test
public func testPushEffectUndoOnError(): Unit {
    let host = newHost()
    let rec = Recorder()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(invokeCmd("i", "run", JNull))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ ctx, _a =>
        ctx.pushEffect(SimpleEffect(rec, "undo-x"))
        throw SdkError("业务失败", "failed")
    }, rec))
    let body = host.replyFor("i")
    @Assert(jAsStr(jObjGet(body, "type")) == Some("err"))
    @Assert(rec.undos.size == 1)
    @Assert(rec.undos[0] == "undo-x")
}

@Test
public func testUndoFailureSupersedesResult(): Unit {
    let host = newHost()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(invokeCmd("i", "run", JNull))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ ctx, _a =>
        ctx.pushEffect(BoomEffect())
        return JStr("done")
    }, Recorder()))
    let body = host.replyFor("i")
    @Assert(jAsStr(jObjGet(body, "type")) == Some("err"))
    @Assert(msgOf(host, 2).contains("Undo 失败"))
}

internal class BoomEffect <: Effect {
    public func apply(_ctx: GuestCtx): Undo {
        return BoomUndo()
    }
}

internal class BoomUndo <: Undo {
    public func run(): Unit {
        throw SdkError("回滚爆炸", "failed")
    }
}

@Test
public func testFeedbackRecord(): Unit {
    let host = newHost()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(invokeCmd("i", "run", JNull))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ ctx, _a =>
        let tags = ArrayList<String>()
        tags.add("perf")
        tags.add("slow")
        ctx.feedback(tags, "耗时偏长")
        return JNull
    }, Recorder()))
    let opFrame = ctxOpOf(host, "feedback")
    let record = jObjGet(jObjGet(opFrame, "op"), "record")
    let tags = match (jAsArr(jObjGet(record, "tags"))) {
        case Some(l) => l
        case None => @Assert(false); ArrayList<JsonValue>()
    }
    @Assert(tags.size == 2)
    @Assert(jAsStr(tags[0]) == Some("perf"))
    @Assert(jAsStr(jObjGet(record, "chain_id")) == Some("chain-9"))
    @Assert(jAsNum(jObjGet(record, "span")) == Some(3.0))
    @Assert(jAsStr(jObjGet(jObjGet(record, "loc"), "ability")) == Some("inst-1"))
}

@Test
public func testRegisterAspectRejected(): Unit {
    let host = newHost()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(invokeCmd("i", "run", JNull))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ ctx, _a =>
        ctx.registerAspect(JNull)
        return JNull
    }, Recorder()))
    let body = host.replyFor("i")
    @Assert(jAsStr(jObjGet(body, "type")) == Some("err"))
    @Assert(msgOf(host, 2).contains("register_aspect"))
}

@Test
public func testCtxShareAndRetain(): Unit {
    let host = newHost()
    host.push(helloCmd("h"))
    host.push(createCmd("c"))
    host.push(invokeCmd("i", "run", JNull))
    host.push(cmdFrame("s", jObjOf([("type", JStr("shutdown"))])))
    host.runServe(specProvider({ ctx, _a =>
        let shared = ctx.share()
        let kept = shared.retain()
        return jObjOf([("shared", JStr(shared.ctxId)), ("kept", JStr(kept.ctxId))])
    }, Recorder()))
    let body = host.replyFor("i")
    @Assert(jAsStr(jObjGet(body, "type")) == Some("ok"))
    @Assert(jAsStr(jObjGet(jObjGet(body, "value"), "shared")) == Some("ctx-2"))
    @Assert(jAsStr(jObjGet(jObjGet(body, "value"), "kept")) == Some("ctx-2"))
}

@Test
public func testServeAbilityBuildProvider(): Unit {
    let decl = loadAbilityYaml(
        "framework_version: \"cangjie@1\"\nid: algo.demo\nstateful: false\ndepends:\n  - id: dep.a\n    alias: a\nmethods:\n  - id: run\n    description: 跑\n")
    let handlers = HashMap<String, Handler>()
    handlers.add("run", { _ctx, args => args })
    let provider = buildProvider(decl, handlers, HashMap<String, (GuestCtx) -> Unit>())
    let m = provider.manifest()
    @Assert(jAsStr(jObjGet(m, "id")) == Some("algo.demo"))
    let methods = match (jAsArr(jObjGet(m, "methods"))) {
        case Some(l) => l
        case None => @Assert(false); ArrayList<JsonValue>()
    }
    @Assert(jAsStr(jObjGet(methods[0], "description")) == Some("跑"))
}

@Test
public func testBuildProviderMissingHandlerThrows(): Unit {
    let decl = loadAbilityYaml("id: algo.demo\nmethods:\n  - id: run\n")
    var threw = false
    try {
        buildProvider(decl, HashMap<String, Handler>(), HashMap<String, (GuestCtx) -> Unit>())
    } catch (e: SdkError) {
        threw = true
    }
    @Assert(threw)
}
