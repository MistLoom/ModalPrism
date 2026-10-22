"""能力作者门面：handlers 字典 + ability.yml 即可运行。"""

from __future__ import annotations

import os
import sys
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
        spec.on(mid, m.get("description") or mid, m.get("input") or {"type": "object"}, handlers[mid])
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
