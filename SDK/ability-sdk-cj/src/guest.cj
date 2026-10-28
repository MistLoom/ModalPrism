// Guest 桥与 serve 入口（对齐 lib/guest.js / guest.py：stdio JSON 行帧、同步泵循环）。
package leaf_ability_sdk

import std.collection.ArrayList
import std.collection.HashMap

// 撤销动作：invoke 边界回滚。
public interface Undo {
    func run(): Unit
}

// 效果：apply 成功后返回 Undo，随 invoke 结束（含异常）倒序执行。
public interface Effect {
    func apply(ctx: GuestCtx): Undo
}

// 事件处理者：mode = message | parallel | pipeline | middleware
public class HandleHandler {
    public let mode: String
    public let around: ?(JsonValue, (JsonValue) -> JsonValue) -> JsonValue
    public let process: ?(JsonValue) -> JsonValue
    public let onEvent: ?(JsonValue) -> Unit

    public init(mode: String, around: ?(JsonValue, (JsonValue) -> JsonValue) -> JsonValue,
        process: ?(JsonValue) -> JsonValue, onEvent: ?(JsonValue) -> Unit) {
        this.mode = mode
        this.around = around
        this.process = process
        this.onEvent = onEvent
    }

    public static func message(onEvent: (JsonValue) -> Unit): HandleHandler {
        return HandleHandler("message", None, None, Some(onEvent))
    }

    public static func parallel(onEvent: (JsonValue) -> Unit): HandleHandler {
        return HandleHandler("parallel", None, None, Some(onEvent))
    }

    public static func pipeline(process: (JsonValue) -> JsonValue): HandleHandler {
        return HandleHandler("pipeline", None, Some(process), None)
    }

    public static func middleware(around: (JsonValue, (JsonValue) -> JsonValue) -> JsonValue): HandleHandler {
        return HandleHandler("middleware", Some(around), None, None)
    }
}

public class HandleReg {
    let bridge: Bridge
    let guestId: String
    let ctxId: String
    let regId: Int64

    init(bridge: Bridge, guestId: String, ctxId: String, regId: Int64) {
        this.bridge = bridge
        this.guestId = guestId
        this.ctxId = ctxId
        this.regId = regId
    }

    public func unregister(): Unit {
        bridge.handlers.remove(guestId)
        try {
            bridge.request(jObjOf([
                ("type", JStr("ctx")),
                ("ctx_id", JStr(ctxId)),
                ("op", jObjOf([
                    ("op", JStr("unhandle")),
                    ("id", JNum(Float64(regId))),
                ])),
            ]))
        } catch (e: Exception) {
            // 宿主已不可达时忽略
        }
    }
}

public class Subscription {
    let bridge: Bridge
    public let sub: String
    let queue: ArrayList<JsonValue>

    init(bridge: Bridge, sub: String) {
        this.bridge = bridge
        this.sub = sub
        this.queue = ArrayList<JsonValue>()
    }

    internal func push(event: JsonValue): Unit {
        queue.add(event)
    }

    public func next(): JsonValue {
        if (queue.size > 0) {
            let ev = queue[0]
            queue.remove(0..1)
            return ev
        }
        while (true) {
            bridge.pumpLine()
            if (queue.size > 0) {
                let ev = queue[0]
                queue.remove(0..1)
                return ev
            }
        }
        throw failSdk("unreachable", "failed")
    }
}

public class HandlerSlot {
    public let handler: HandleHandler
    public let ctxId: String

    init(handler: HandleHandler, ctxId: String) {
        this.handler = handler
        this.ctxId = ctxId
    }
}

public class GuestCtx {
    let bridge: Bridge
    public let ctxId: String
    public let instance: ?String
    public let method: ?String
    public let chainId: ?String
    public let span: Int64
    let undos: ArrayList<Undo>

    init(bridge: Bridge, ctxId: String, instance: ?String, method: ?String,
        chainId: ?String, span: Int64, undos: ArrayList<Undo>) {
        this.bridge = bridge
        this.ctxId = ctxId
        this.instance = instance
        this.method = method
        this.chainId = chainId
        this.span = span
        this.undos = undos
    }

    // 发送 Ctx 帧并等待宿主应答。
    public func ctxOp(op: JsonValue): JsonValue {
        let frame = bridge.request(jObjOf([
            ("type", JStr("ctx")),
            ("ctx_id", JStr(ctxId)),
            ("op", op),
        ]))
        return okValue(frame)
    }

    public func share(): GuestCtx {
        try {
            let v = ctxOp(shareOp())
            let sharedId = jAsStr(jObjGet(v, "ctx_id")) ?? ctxId
            return bridge.createGuestCtx(sharedId, instance, None, chainId, span, ArrayList<Undo>())
        } catch (e: Exception) {
            return bridge.createGuestCtx(ctxId, instance, None, chainId, span, ArrayList<Undo>())
        }
    }

    func shareOp(): JsonValue {
        return jObjOf([("op", JStr("share"))])
    }

    public func retain(): GuestCtx {
        return bridge.createGuestCtx(ctxId, instance, method, chainId, span, undos)
    }

    public func chainStatus(chainIdOpt: ?String): ?JsonValue {
        let op = ArrayList<(String, JsonValue)>()
        op.add(("op", JStr("chain_status")))
        match (chainIdOpt) {
            case Some(cid) => op.add(("chain_id", JStr(cid)))
            case None => op.add(("chain_id", JNull))
        }
        try {
            return Some(ctxOp(jObjOf(op.toArray())))
        } catch (e: Exception) {
            return None
        }
    }

    public func call(idOrAlias: String, methodName: String, args: JsonValue): JsonValue {
        return ctxOp(jObjOf([
            ("op", JStr("call")),
            ("id_or_alias", JStr(idOrAlias)),
            ("method", JStr(methodName)),
            ("args", args),
        ]))
    }

    public func dep(idOrAlias: String): String {
        let v = ctxOp(jObjOf([
            ("op", JStr("dep")),
            ("id_or_alias", JStr(idOrAlias)),
        ]))
        return match (jAsStr(v)) {
            case Some(s) => s
            case None => throw failSdk("dep 返回非字符串", "failed")
        }
    }

    public func catalog(): JsonValue {
        try {
            return ctxOp(jObjOf([("op", JStr("catalog"))]))
        } catch (e: Exception) {
            return JArr(ArrayList<JsonValue>())
        }
    }

    // 命中返回 Some(value)，未命中 None。
    public func localGet(key: String): ?JsonValue {
        let v = ctxOp(jObjOf([
            ("op", JStr("local_get")),
            ("key", JStr(key)),
        ]))
        let hit = jAsBool(jObjGet(v, "hit")) ?? false
        if (hit) {
            return Some(jObjGet(v, "value"))
        }
        return None
    }

    public func localSet(key: String, value: JsonValue): Unit {
        ctxOp(jObjOf([
            ("op", JStr("local_set")),
            ("key", JStr(key)),
            ("value", value),
        ]))
    }

    public func emit(name: String, payload: JsonValue): Unit {
        ctxOp(jObjOf([
            ("op", JStr("emit")),
            ("name", JStr(name)),
            ("payload", payload),
        ]))
    }

    public func load(spec: JsonValue): String {
        let v = ctxOp(jObjOf([
            ("op", JStr("load")),
            ("spec", spec),
        ]))
        return match (jAsStr(jObjGet(v, "instance"))) {
            case Some(s) => s
            case None => throw failSdk("load 未返回 instance", "failed")
        }
    }

    public func unload(instanceName: String): Unit {
        ctxOp(jObjOf([
            ("op", JStr("unload")),
            ("instance", JStr(instanceName)),
        ]))
    }

    public func subscribe(filter: JsonValue): Subscription {
        let v = ctxOp(jObjOf([
            ("op", JStr("subscribe")),
            ("filter", filter),
        ]))
        let subId = jAsStr(jObjGet(v, "sub"))
        let subIdStr = match (subId) {
            case Some(s) => s
            case None => throw failSdk("Subscribe 未返回 sub", "failed")
        }
        if (subIdStr == "") {
            throw failSdk("Subscribe 未返回 sub", "failed")
        }
        let sub = Subscription(bridge, subIdStr)
        bridge.subs.add(subIdStr, sub)
        return sub
    }

    public func handle(filter: JsonValue, handler: HandleHandler, prepend: Bool): HandleReg {
        let guestId = "h${bridge.nextGuestId()}"
        bridge.handlers.add(guestId, HandlerSlot(handler, ctxId))
        let v = ctxOp(jObjOf([
            ("op", JStr("handle")),
            ("filter", filter),
            ("prepend", JBool(prepend)),
            ("mode", JStr(handler.mode)),
            ("guest_id", JStr(guestId)),
        ]))
        let regId = jAsNum(jObjGet(v, "id"))
        let regIdNum = match (regId) {
            case Some(n) => n
            case None => throw failSdk("Handle 未返回 id", "failed")
        }
        if (Float64(Int64(regIdNum)) != regIdNum) {
            throw failSdk("Handle 未返回 id", "failed")
        }
        return HandleReg(bridge, guestId, ctxId, Int64(regIdNum))
    }

    public func pushEffect(effect: Effect): Unit {
        let undo = effect.apply(this)
        undos.add(undo)
    }

    public func feedback(tags: ArrayList<String>, desc: String): Unit {
        submitFeedback(this, tags, desc)
    }

    // Guest 不支持进程内切面：与 Node/Python 一致显式拒绝。
    public func registerAspect(_aspect: JsonValue): Unit {
        throw failSdk("Guest 不支持 register_aspect", "failed")
    }

    public func withProtocolFilter(_kind: String): GuestCtx {
        return retain()
    }
}

// 同步桥：request 期间泵入站帧，事件/.run_handle 就地分发，命令帧入队。
public class Bridge {
    public let writeLine: (String) -> Unit
    public let input: () -> ?String
    public var pending: HashMap<String, ?JsonValue>
    public var subs: HashMap<String, Subscription>
    public var handlers: HashMap<String, HandlerSlot>
    public var inbox: ArrayList<JsonValue>
    var guestIdSeq: Int64

    init(input: () -> ?String, writeLine: (String) -> Unit) {
        this.input = input
        this.writeLine = writeLine
        this.pending = HashMap<String, ?JsonValue>()
        this.subs = HashMap<String, Subscription>()
        this.handlers = HashMap<String, HandlerSlot>()
        this.inbox = ArrayList<JsonValue>()
        this.guestIdSeq = 1
    }

    func nextGuestId(): Int64 {
        let n = guestIdSeq
        guestIdSeq += 1
        return n
    }

    public func send(frame: JsonValue): Unit {
        writeLine(encodeFrame(frame))
    }

    public func request(body: JsonValue): JsonValue {
        let id = nextFrameId()
        let frame = jObjOf([
            ("id", JStr(id)),
            ("body", body),
        ])
        pending.add(id, None)
        send(frame)
        var arrived = false
        while (!arrived) {
            match (pending.get(id)) {
                case Some(inner) =>
                    match (inner) {
                        case Some(_) => arrived = true
                        case None => pumpLine()
                    }
                case None => pumpLine()
            }
        }
        let reply = pending.get(id)
        pending.remove(id)
        return match (reply) {
            case Some(inner) =>
                match (inner) {
                    case Some(f) => f
                    case None => throw failSdk("应答丢失", "failed")
                }
            case None => throw failSdk("应答丢失", "failed")
        }
    }

    public func pumpLine(): Unit {
        let line = input()
        match (line) {
            case Some(l) => ingestLine(l)
            case None => throw failSdk("宿主 stdin 已关闭", "failed")
        }
    }

    func ingestLine(line: String): Unit {
        if (line.trimAscii() == "") {
            return
        }
        let frame = decodeFrame(line)
        // 应答帧：reply_to 指向等待中的请求
        let replyTo = jAsStr(jObjGet(frame, "reply_to"))
        if (let Some(rid) <- replyTo) {
            if (pending.contains(rid)) {
                pending.add(rid, Some(frame))
                return
            }
        }
        let body = jObjGet(frame, "body")
        let btype = jAsStr(jObjGet(body, "type"))
        if (btype == Some("event") || btype == Some("run_handle")) {
            dispatchSide(frame)
            return
        }
        // 等待应答时读到的命令帧入队，回到主循环再按序执行。
        inbox.add(frame)
    }

    func dispatchSide(frame: JsonValue): Unit {
        let cmdId = jAsStr(jObjGet(frame, "id")) ?? ""
        let body = jObjGet(frame, "body")
        let btype = jAsStr(jObjGet(body, "type"))
        if (btype == Some("event")) {
            let subId = jAsStr(jObjGet(body, "sub"))
            if (let Some(sid) <- subId) {
                if (let Some(sub) <- subs.get(sid)) {
                    sub.push(jObjGet(body, "event"))
                }
            }
            return
        }
        if (btype == Some("run_handle")) {
            let guestId = jAsStr(jObjGet(body, "guest_id"))
            let event = jObjGet(body, "event")
            let nextToken = jAsStr(jObjGet(body, "next_token"))
            try {
                let value = runGuestHandle(guestId ?? "", event, nextToken)
                send(frameReply(cmdId, jObjOf([
                    ("type", JStr("ok")),
                    ("value", value),
                ])))
            } catch (e: SdkError) {
                send(frameReply(cmdId, wireErrorBody(e)))
            } catch (e: Exception) {
                send(frameReply(cmdId, wireErrorBody(SdkError("${e.message}", "failed"))))
            }
        }
    }

    public func runGuestHandle(guestId: String, event: JsonValue, nextToken: ?String): JsonValue {
        let slot = handlers.get(guestId)
        if (let Some(s) <- slot) {
            return runHandleSlot(s, event, nextToken)
        }
        throw SdkError("事件处理者已卸", "not_found")
    }

    func runHandleSlot(slot: HandlerSlot, event: JsonValue, nextToken: ?String): JsonValue {
        let h = slot.handler
        let mode = h.mode
        if (mode == "middleware") {
            if (let Some(around) <- h.around) {
                let nextFn = { ev: JsonValue =>
                    let token = nextToken
                    match (token) {
                        case Some(t) =>
                            if (t == "") {
                                return ev
                            }
                            return okValue(bridgeRequest(slot.ctxId, eventNextOp(t, ev)))
                        case None => return ev
                    }
                }
                return around(event, nextFn)
            }
            throw failSdk("middleware 缺少 around", "failed")
        }
        if (mode == "pipeline") {
            if (let Some(process) <- h.process) {
                return process(event)
            }
            throw failSdk("pipeline 缺少 process", "failed")
        }
        if (mode == "message" || mode == "parallel") {
            if (let Some(onEvent) <- h.onEvent) {
                onEvent(event)
                return event
            }
            throw failSdk("${mode} 缺少 onEvent", "failed")
        }
        throw failSdk("未知事件模式: ${mode}", "failed")
    }

    func bridgeRequest(ctxId: String, op: JsonValue): JsonValue {
        return request(jObjOf([
            ("type", JStr("ctx")),
            ("ctx_id", JStr(ctxId)),
            ("op", op),
        ]))
    }

    func eventNextOp(token: String, ev: JsonValue): JsonValue {
        return jObjOf([
            ("op", JStr("event_next")),
            ("token", JStr(token)),
            ("event", ev),
        ])
    }

    public func createGuestCtx(ctxId: String, instance: ?String, method: ?String,
        chainId: ?String, span: Int64, undos: ArrayList<Undo>): GuestCtx {
        return GuestCtx(this, ctxId, instance, method, chainId, span, undos)
    }
}

public func wireErrorBody(e: SdkError): JsonValue {
    return jObjOf([
        ("type", JStr("err")),
        ("error", jObjOf([
            ("kind", JStr(e.kind)),
            ("message", JStr("${e.message}")),
        ])),
    ])
}

// invoke / lifecycle 边界的 undo 回滚：倒序执行，undo 失败升级为返回错误。
public func runUndos(undos: ArrayList<Undo>, origin: ?SdkError): Unit {
    var acc = origin
    while (undos.size > 0) {
        let u = undos[undos.size - 1]
        undos.remove(undos.size - 1..undos.size)
        try {
            u.run()
        } catch (e: Exception) {
            match (acc) {
                case None => acc = Some(SdkError("Undo 失败: ${e.message}", "failed"))
                case Some(_) => ()
            }
        }
    }
    match (acc) {
        case Some(err) => throw err
        case None => return
    }
}

// serve 主循环（可注入 IO，便于测试桩模拟宿主侧帧）。
public func serveWithIO(provider: AbilityProvider, input: () -> ?String,
    write: (String) -> Unit): Unit {
    let bridge = Bridge(input, write)
    var ability: ?AbilityInstance = None
    while (true) {
        let next = nextCommand(bridge)
        if (let Some(frame) <- next) {
            let cmdId = jAsStr(jObjGet(frame, "id")) ?? ""
            let body = jObjGet(frame, "body")
            let btype = jAsStr(jObjGet(body, "type"))
            if (btype == Some("shutdown")) {
                return
            }
            if (btype == Some("hello")) {
                bridge.send(frameReply(cmdId, jObjOf([
                    ("type", JStr("manifest")),
                    ("manifest", provider.manifest()),
                ])))
                continue
            }
            if (btype == Some("create")) {
                try {
                    ability = Some(provider.create(jObjGet(body, "config")))
                    bridge.send(frameReply(cmdId, jObjOf([
                        ("type", JStr("ok")),
                        ("value", JNull),
                    ])))
                } catch (e: SdkError) {
                    ability = None
                    bridge.send(frameReply(cmdId, wireErrorBody(e)))
                } catch (e: Exception) {
                    ability = None
                    bridge.send(frameReply(cmdId, wireErrorBody(SdkError("${e.message}", "failed"))))
                }
                continue
            }
            if (btype == Some("invoke") || btype == Some("lifecycle")) {
                handleCommandFrame(bridge, ability, cmdId, body)
                continue
            }
            continue
        }
        return
    }
}

func handleCommandFrame(bridge: Bridge, ability: ?AbilityInstance, cmdId: String,
    body: JsonValue): Unit {
    let btype = jAsStr(jObjGet(body, "type"))
    var slot: ?AbilityInstance = ability
    if (let None <- slot) {
        bridge.send(frameReply(cmdId, jObjOf([
            ("type", JStr("err")),
            ("error", jObjOf([
                ("kind", JStr("failed")),
                ("message", JStr("尚未 Create")),
            ])),
        ])))
        return
    }
    let inst = match (slot) {
        case Some(a) => a
        case None => return
    }
    let chainId = jAsStr(jObjGet(body, "chain_id"))
    let instanceName = jAsStr(jObjGet(body, "instance"))
    let spanNum = jAsNum(jObjGet(body, "span")) ?? 0.0
    let undos = ArrayList<Undo>()
    let ctx = bridge.createGuestCtx(cmdId, instanceName, None, chainId, Int64(spanNum), undos)
    try {
        if (btype == Some("invoke")) {
            let method = jAsStr(jObjGet(body, "method")) ?? ""
            let ctx2 = bridge.createGuestCtx(cmdId, instanceName, Some(method), chainId,
                Int64(spanNum), undos)
            let result = inst.invoke(method, ctx2, jObjGet(body, "args"))
            runUndos(undos, None)
            bridge.send(frameReply(cmdId, jObjOf([
                ("type", JStr("ok")),
                ("value", result),
            ])))
        } else {
            let phase = jAsStr(jObjGet(body, "phase")) ?? ""
            inst.onLifecycle(phase, ctx)
            runUndos(undos, None)
            bridge.send(frameReply(cmdId, jObjOf([
                ("type", JStr("ok")),
                ("value", JNull),
            ])))
        }
    } catch (e: SdkError) {
        replyAfterUndo(bridge, cmdId, undos, Some(e))
    } catch (e: Exception) {
        replyAfterUndo(bridge, cmdId, undos, Some(SdkError("${e.message}", "failed")))
    }
}

func replyAfterUndo(bridge: Bridge, cmdId: String, undos: ArrayList<Undo>,
    err: ?SdkError): Unit {
    try {
        runUndos(undos, err)
        match (err) {
            case Some(e) => bridge.send(frameReply(cmdId, wireErrorBody(e)))
            case None => ()
        }
    } catch (ue: SdkError) {
        bridge.send(frameReply(cmdId, wireErrorBody(ue)))
    } catch (e: Exception) {
        bridge.send(frameReply(cmdId, wireErrorBody(SdkError("${e.message}", "failed"))))
    }
}

// 取下一条命令帧：收件箱优先；等待应答时读到的命令帧已在收件箱。
func nextCommand(bridge: Bridge): ?JsonValue {
    while (true) {
        if (bridge.inbox.size > 0) {
            let frame = bridge.inbox[0]
            bridge.inbox.remove(0..1)
            return Some(frame)
        }
        let line = bridge.input()
        match (line) {
            case None => return None
            case Some(l) =>
                if (l.trimAscii() == "") {
                    continue
                }
                let frame = decodeFrame(l)
                let replyTo = jAsStr(jObjGet(frame, "reply_to"))
                if (let Some(rid) <- replyTo) {
                    if (bridge.pending.contains(rid)) {
                        bridge.pending.add(rid, frame)
                        continue
                    }
                }
                let body = jObjGet(frame, "body")
                let btype = jAsStr(jObjGet(body, "type"))
                if (btype == Some("event") || btype == Some("run_handle")) {
                    bridge.dispatchSide(frame)
                    continue
                }
                return Some(frame)
        }
        }
        return None
}
