"""能力作者门面：handlers 字典 + ability.yml 即可运行。"""

from __future__ import annotations

import os
import sys
import threading
import time
import uuid
from pathlib import Path
from typing import Any, Awaitable, Callable, Dict

from .guest import serve_stdio
from .spec import AbilitySpec, FRAMEWORK_VERSION
from .yaml_lite import load_ability_yaml, schema_version

Handler = Callable[[Any, Any], Awaitable[Any] | Any]
LifecycleHooks = Dict[str, Handler]


def serve_ability(
    handlers: Dict[str, Handler],
    *,
    root: str | Path | None = None,
    lifecycle: LifecycleHooks | None = None,
    on_create: Callable[[Any], None] | None = None,
) -> None:
    """从 ability.yml 与 handlers 启动 stdio guest。"""
    ability_root = _resolve_root(root)
    decl = load_ability_yaml(ability_root)
    if schema_version(decl["framework_version"]) != FRAMEWORK_VERSION:
        raise RuntimeError("ability.yml framework_version 与 SDK 不匹配")
    provider = build_provider(decl, handlers, lifecycle or {}, on_create=on_create)
    serve_stdio(provider)


def build_provider(
    decl: dict,
    handlers: Dict[str, Handler],
    lifecycle: LifecycleHooks,
    *,
    on_create: Callable[[Any], None] | None = None,
) -> Any:
    spec = AbilitySpec.must(decl["id"])
    if decl.get("stateful"):
        spec.stateful = True
    for dep in decl.get("depends", []):
        if dep.get("alias"):
            spec.depends_alias(dep["id"], dep["alias"])
    for ev in decl.get("events", []):
        if ev.get("dir") == "emit":
            spec.event_emit(ev["name"], ev.get("description", ""), ev.get("payload_schema"))
    for m in decl["methods"]:
        mid = m["id"]
        if mid not in handlers:
            raise RuntimeError(f"缺少方法处理器: {mid}")
        spec.on(
            mid,
            m.get("description") or mid,
            m.get("input") or {"type": "object"},
            with_long_task(handlers[mid]),
        )
    base = spec.into_provider()

    class Provider:
        def manifest(self) -> dict:
            return base.manifest()

        async def create(self, config):
            if on_create:
                on_create(config)
            ability = await base.create(config)

            class Ability:
                def manifest(self) -> dict:
                    return ability.manifest()

                async def on_lifecycle(self, phase, ctx):
                    hook = lifecycle.get(phase)
                    if hook:
                        result = hook(ctx)
                        if hasattr(result, "__await__"):
                            await result
                    await ability.on_lifecycle(phase, ctx)

                async def invoke(self, method, ctx, args):
                    return await ability.invoke(method, ctx, args)

            return Ability()

    return Provider()


def handler(fn: Handler) -> Handler:
    return fn



def pending(work) -> dict:
    """标记长任务：方法立即返回 `{task_handle}`，调用方以同一方法 + `{handle}` 轮询。

    `work` 可以是可调用对象（推荐，在后台线程执行）、awaitable，或已完成值。
    """
    return {"$pending": work}


def with_long_task(fn: Handler) -> Handler:
    """协议级长任务包装：拦截 `{handle}` 轮询；识别 `pending(...)` 返回值。"""
    tasks: dict[str, dict[str, Any]] = {}
    lock = threading.Lock()
    seq = 0

    def _settle(handle: str, work) -> None:
        try:
            value = work() if callable(work) else work
            if hasattr(value, "__await__"):
                import asyncio

                value = asyncio.run(value)
            with lock:
                tasks[handle] = {"state": "done", "data": value}
        except BaseException as e:  # noqa: BLE001
            msg = getattr(e, "message", None) or str(e)
            with lock:
                tasks[handle] = {"state": "failed", "error": msg}

    def wrapped(ctx, args):
        nonlocal seq
        if isinstance(args, dict) and isinstance(args.get("handle"), str):
            handle = args["handle"]
            with lock:
                task = tasks.get(handle)
            if task is None:
                return {"state": "not_found", "handle": handle}
            if task["state"] == "running":
                return {"state": "running", "handle": handle}
            with lock:
                tasks.pop(handle, None)
            if task["state"] == "done":
                return {"state": "done", "data": task.get("data")}
            return {"state": "failed", "error": task.get("error", "failed")}

        result = fn(ctx, args)
        if isinstance(result, dict) and "$pending" in result:
            seq += 1
            handle = f"task-{uuid.uuid4().hex[:10]}-{seq}"
            with lock:
                tasks[handle] = {"state": "running"}
            threading.Thread(
                target=_settle, args=(handle, result["$pending"]), daemon=True
            ).start()
            return {"task_handle": handle}
        return result

    return wrapped


def _resolve_root(explicit: str | Path | None) -> Path:
    if explicit is not None:
        return Path(explicit)
    env = os.environ.get("LEAF_ABILITY_ROOT")
    if env:
        return Path(env)
    start = Path(sys.argv[0]).resolve().parent if sys.argv[0] else Path.cwd()
    dir_ = start
    while True:
        if (dir_ / "ability.yml").is_file():
            return dir_
        if dir_.parent == dir_:
            break
        dir_ = dir_.parent
    return Path.cwd()
