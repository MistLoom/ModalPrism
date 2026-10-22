from __future__ import annotations

import inspect
import sys
import uuid
from typing import Any, Callable

from .feedback import submit_feedback
from .protocol import decode_frame, encode_frame, frame_reply, ok_value


def _wire_error(err: BaseException) -> dict:
    kind = getattr(err, "kind", "failed")
    return {"type": "err", "error": {"kind": kind, "message": str(err)}}


class _Subscription:
    def __init__(self, sub_id: str, bridge: "_SyncBridge") -> None:
        self._sub = sub_id
        self._bridge = bridge
        self._queue: list[Any] = []
        self._waiters: list[Callable[[Any], None]] = []

    def push(self, event: Any) -> None:
        if self._waiters:
            self._waiters.pop(0)(event)
        else:
            self._queue.append(event)

    def next(self) -> Any:
        if self._queue:
            return self._queue.pop(0)
        box: list[Any] = []

        def wake(ev: Any) -> None:
            box.append(ev)

        self._waiters.append(wake)
        while not box:
            self._bridge.pump_line()
        return box[0]


class _SyncBridge:
    def __init__(self, write_line) -> None:
        self._write = write_line
        self.pending: dict[str, Any] = {}
        self.subs: dict[str, _Subscription] = {}
        self.handlers: dict[str, dict[str, Any]] = {}
        self.inbox: list[dict] = []
        self._guest_id_seq = 1

    def send(self, frame: dict) -> None:
        self._write(encode_frame(frame))

    def pump_line(self) -> None:
        line = sys.stdin.readline()
        if not line:
            raise RuntimeError("宿主 stdin 已关闭")
        self._ingest_line(line)

    def _ingest_line(self, line: str) -> None:
        if not line.strip():
            return
        if self.handle_line(line):
            return
        frame = decode_frame(line)
        body = frame.get("body") or {}
        if body.get("type") in ("event", "run_handle"):
            self._dispatch_side_frame(frame)
            return
        # 等待应答时读到的命令帧入队，回到主循环再按序执行。
        self.inbox.append(frame)

    def request(self, body: dict) -> Any:
        frame = {"id": str(uuid.uuid4()), "body": body}
        self.pending[frame["id"]] = None
        self.send(frame)
        rid = frame["id"]
        while self.pending.get(rid) is None:
            self.pump_line()
        reply = self.pending.pop(rid)
        return ok_value(reply)

    def handle_line(self, line: str) -> bool:
        if not line.strip():
            return False
        frame = decode_frame(line)
        rid = frame.get("reply_to")
        if rid and rid in self.pending and self.pending[rid] is None:
            self.pending[rid] = frame
            return True
        return False

    def _dispatch_side_frame(self, frame: dict) -> None:
        body = frame.get("body") or {}
        btype = body.get("type")
        if btype == "event":
            sub = self.subs.get(body.get("sub"))
            if sub:
                sub.push(body.get("event"))
            return
        if btype == "run_handle":
            cmd_id = frame["id"]
            try:
                value = self.run_guest_handle(
                    body["guest_id"],
                    body["event"],
                    body.get("next_token"),
                )
                self.send(frame_reply(cmd_id, {"type": "ok", "value": value}))
            except Exception as e:
                self.send(frame_reply(cmd_id, _wire_error(e)))

    def run_guest_handle(self, guest_id: str, event: Any, next_token: str | None) -> Any:
        slot = self.handlers.get(guest_id)
        if not slot:
            err = RuntimeError("事件处理者已卸")
            err.kind = "not_found"  # type: ignore[attr-defined]
            raise err
        h = slot["handler"]
        mode = getattr(h, "mode", "message")
        if mode == "middleware":
            around = getattr(h, "around", None)
            if not callable(around):
                raise RuntimeError("middleware 缺少 around")

            def next_fn(ev: Any) -> Any:
                if not next_token:
                    return ev
                return self.request(
                    {
                        "type": "ctx",
                        "ctx_id": slot["ctx_id"],
                        "op": {"op": "event_next", "token": next_token, "event": ev},
                    },
                )

            return _await_maybe(around(event, next_fn))
        if mode == "pipeline":
            process = getattr(h, "process", None)
            if not callable(process):
                raise RuntimeError("pipeline 缺少 process")
            return _await_maybe(process(event))
        if mode in ("message", "parallel"):
            on_event = getattr(h, "on_event", None)
            if not callable(on_event):
                raise RuntimeError(f"{mode} 缺少 on_event")
            _await_maybe(on_event(event))
            return event
        raise RuntimeError(f"未知事件模式: {mode}")

    def create_guest_ctx(
        self,
        ctx_id: str,
        instance: Any,
        method: Any,
        chain_id: Any,
        span: Any,
        undos: list,
    ) -> Any:
        bridge = self

        class Ctx:
            def ctx_op(self, op: dict) -> Any:
                return bridge.request({"type": "ctx", "ctx_id": ctx_id, "op": op})

            def share(self):
                new_id = ctx_id
                try:
                    v = self.ctx_op({"op": "share"})
                    if isinstance(v, dict) and isinstance(v.get("ctx_id"), str):
                        new_id = v["ctx_id"]
                except Exception:
                    new_id = ctx_id
                return bridge.create_guest_ctx(new_id, instance, None, chain_id, span, [])

            def retain(self):
                return bridge.create_guest_ctx(ctx_id, instance, method, chain_id, span, undos)

            def chain_status(self, chain_id_opt=None):
                try:
                    return self.ctx_op({"op": "chain_status", "chain_id": chain_id_opt})
                except Exception:
                    return None

            def call(self, id_or_alias: str, method_name: str, args) -> Any:
                return self.ctx_op(
                    {
                        "op": "call",
                        "id_or_alias": id_or_alias,
                        "method": method_name,
                        "args": args,
                    },
                )

            def dep(self, id_or_alias: str) -> str:
                v = self.ctx_op({"op": "dep", "id_or_alias": id_or_alias})
                if not isinstance(v, str):
                    raise RuntimeError("dep 返回非字符串")
                return v

            def catalog(self):
                try:
                    return self.ctx_op({"op": "catalog"})
                except Exception:
                    return []

            def local_get(self, key: str):
                v = self.ctx_op({"op": "local_get", "key": key})
                if isinstance(v, dict) and v.get("hit"):
                    return v.get("value")
                return None

            def local_set(self, key: str, value) -> None:
                self.ctx_op({"op": "local_set", "key": key, "value": value})

            def emit(self, name: str, payload) -> None:
                self.ctx_op({"op": "emit", "name": name, "payload": payload})

            def load(self, spec) -> Any:
                v = self.ctx_op({"op": "load", "spec": spec})
                instance_id = v.get("instance") if isinstance(v, dict) else None
                if not isinstance(instance_id, str):
                    raise RuntimeError("load 未返回 instance")
                return instance_id

            def unload(self, instance) -> None:
                self.ctx_op({"op": "unload", "instance": instance})

            def subscribe(self, filter: dict):
                v = self.ctx_op({"op": "subscribe", "filter": filter})
                sub_id = v.get("sub") if isinstance(v, dict) else None
                if not isinstance(sub_id, str) or not sub_id:
                    raise RuntimeError("Subscribe 未返回 sub")
                sub = _Subscription(sub_id, bridge)
                bridge.subs[sub_id] = sub
                return sub

            def handle(self, filter: dict, handler, opts: dict | None = None):
                opts = opts or {}
                guest_id = f"h{bridge._guest_id_seq}"
                bridge._guest_id_seq += 1
                bridge.handlers[guest_id] = {"handler": handler, "ctx_id": ctx_id}
                v = self.ctx_op(
                    {
                        "op": "handle",
                        "filter": filter,
                        "prepend": bool(opts.get("prepend")),
                        "mode": getattr(handler, "mode", "message"),
                        "guest_id": guest_id,
                    },
                )
                reg_id = v.get("id") if isinstance(v, dict) else None
                if isinstance(reg_id, bool) or not isinstance(reg_id, int):
                    raise RuntimeError("Handle 未返回 id")

                class Reg:
                    def unregister(self) -> None:
                        bridge.handlers.pop(guest_id, None)
                        try:
                            bridge.request(
                                {
                                    "type": "ctx",
                                    "ctx_id": ctx_id,
                                    "op": {"op": "unhandle", "id": reg_id},
                                },
                            )
                        except Exception:
                            pass

                return Reg()

            def push_effect(self, effect) -> None:
                undo = effect.apply(self)
                if inspect.isawaitable(undo):
                    undo = _run_async(undo)
                undos.append(undo)

            def feedback(self, tags, desc: str) -> None:
                submit_feedback(self, tags, desc)

            def register_aspect(self, _aspect):
                err = RuntimeError("Guest 不支持 register_aspect")
                err.kind = "failed"  # type: ignore[attr-defined]
                raise err

            def with_protocol_filter(self, _kind):
                return self.retain()

        ctx = Ctx()
        ctx.instance = instance
        ctx.method = method
        ctx.chain_id = chain_id
        ctx.span = span
        return ctx


def _run_async(coro):
    import asyncio

    return asyncio.run(coro)


def _await_maybe(result: Any) -> Any:
    if inspect.isawaitable(result):
        return _run_async(result)
    return result


def _run_undos(undos: list, origin: Any) -> Any:
    acc = origin
    while undos:
        u = undos.pop()
        try:
            if u and callable(getattr(u, "run", None)):
                _await_maybe(u.run())
        except Exception as e:
            if not isinstance(acc, BaseException):
                err = RuntimeError(f"Undo 失败: {e}")
                err.kind = "failed"  # type: ignore[attr-defined]
                acc = err
    if isinstance(acc, BaseException):
        raise acc
    return acc


def _configure_stdio() -> None:
    for stream in (sys.stdin, sys.stdout):
        reconfigure = getattr(stream, "reconfigure", None)
        if reconfigure:
            reconfigure(encoding="utf-8", newline="\n")


def _next_command(bridge: _SyncBridge) -> dict | None:
    while True:
        if bridge.inbox:
            return bridge.inbox.pop(0)
        line = sys.stdin.readline()
        if not line:
            return None
        if bridge.handle_line(line):
            continue
        if not line.strip():
            continue
        frame = decode_frame(line)
        body = frame.get("body") or {}
        if body.get("type") in ("event", "run_handle"):
            bridge._dispatch_side_frame(frame)
            continue
        return frame


def serve_stdio(provider) -> None:
    _configure_stdio()

    def write_line(line: str) -> None:
        sys.stdout.write(line)
        sys.stdout.flush()

    bridge = _SyncBridge(write_line)
    ability = None

    while True:
        frame = _next_command(bridge)
        if frame is None:
            break
        body = frame["body"]
        btype = body.get("type")
        if btype == "shutdown":
            break
        if btype in ("event", "run_handle"):
            bridge._dispatch_side_frame(frame)
            continue
        if btype == "hello":
            bridge.send(
                frame_reply(
                    frame["id"],
                    {"type": "manifest", "manifest": provider.manifest()},
                ),
            )
        elif btype == "create":
            try:
                ability = _run_async(provider.create(body.get("config")))
                bridge.send(frame_reply(frame["id"], {"type": "ok", "value": None}))
            except Exception as e:
                bridge.send(frame_reply(frame["id"], _wire_error(e)))
        elif btype == "invoke":
            cmd_id = frame["id"]
            if ability is None:
                bridge.send(
                    frame_reply(
                        cmd_id,
                        {
                            "type": "err",
                            "error": {"kind": "failed", "message": "尚未 Create"},
                        },
                    ),
                )
                continue
            undos: list = []
            ctx = bridge.create_guest_ctx(
                cmd_id,
                body["instance"],
                body["method"],
                body["chain_id"],
                body["span"],
                undos,
            )
            reply = None
            try:
                result = ability.invoke(body["method"], ctx, body["args"])
                if inspect.isawaitable(result):
                    result = _run_async(result)
                value = _run_undos(undos, result)
                reply = {"type": "ok", "value": value}
            except Exception as e:
                try:
                    _run_undos(undos, e)
                except Exception as ue:
                    reply = _wire_error(ue)
                if reply is None:
                    reply = _wire_error(e)
            bridge.send(frame_reply(cmd_id, reply))
        elif btype == "lifecycle":
            cmd_id = frame["id"]
            if ability is None:
                bridge.send(
                    frame_reply(
                        cmd_id,
                        {
                            "type": "err",
                            "error": {"kind": "failed", "message": "尚未 Create"},
                        },
                    ),
                )
                continue
            undos: list = []
            ctx = bridge.create_guest_ctx(
                cmd_id,
                body["instance"],
                None,
                body["chain_id"],
                body["span"],
                undos,
            )
            try:
                phase = body["phase"]
                result = ability.on_lifecycle(phase, ctx)
                if inspect.isawaitable(result):
                    _run_async(result)
                bridge.send(frame_reply(cmd_id, {"type": "ok", "value": None}))
            except Exception as e:
                bridge.send(frame_reply(cmd_id, _wire_error(e)))
