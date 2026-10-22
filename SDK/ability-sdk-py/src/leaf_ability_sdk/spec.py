from __future__ import annotations

from typing import Any, Awaitable, Callable

FRAMEWORK_VERSION = "1"

Handler = Callable[[Any, Any], Awaitable[Any]]


class AbilitySpec:
    def __init__(self, id: str) -> None:
        self.id = id
        self.depends: list[dict] = []
        self.events: list[dict] = []
        self.handlers: dict[str, tuple[str, dict, Handler]] = {}
        self.stateful = False
        self.config_schema: Any = None

    @classmethod
    def must(cls, id: str) -> AbilitySpec:
        return cls(id)

    def depends_alias(self, ability_id: str, alias: str) -> AbilitySpec:
        self.depends.append(
            {"id": ability_id, "alias": alias, "optional": False},
        )
        return self

    def event_emit(
        self,
        name: str,
        description: str = "",
        payload_schema: dict | None = None,
    ) -> AbilitySpec:
        self.events.append(
            {
                "name": name,
                "dir": "emit",
                "description": description,
                "payload_schema": payload_schema or {"type": "object"},
            },
        )
        return self

    def on(
        self,
        method_id: str,
        description: str,
        input_schema: dict,
        handler: Handler,
    ) -> AbilitySpec:
        self.handlers[method_id] = (description, input_schema, handler)
        return self

    def manifest(self) -> dict:
        return {
            "id": self.id,
            "depends": self.depends,
            "stateful": self.stateful,
            "config_schema": self.config_schema,
            "methods": [
                {
                    "id": mid,
                    "description": desc,
                    "input_schema": schema,
                    "output_schema": None,
                }
                for mid, (desc, schema, _) in self.handlers.items()
            ],
            "events": self.events,
        }

    def into_provider(self) -> dict:
        spec = self

        class Ability:
            def manifest(self) -> dict:
                return spec.manifest()

            async def on_lifecycle(self, _phase, _ctx) -> None:
                return None

            async def invoke(self, method: str, ctx, args) -> Any:
                if method not in spec.handlers:
                    raise RuntimeError(f"未知方法: {method}")
                _, _, handler = spec.handlers[method]
                result = handler(ctx, args)
                if hasattr(result, "__await__"):
                    return await result
                return result

        class Provider:
            def manifest(self) -> dict:
                return spec.manifest()

            async def create(self, _config) -> Ability:
                return Ability()

        return Provider()
