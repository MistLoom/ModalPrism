// AbilitySpec 与 Provider 形状（对齐 lib/spec.js）。
package leaf_ability_sdk

import std.collection.ArrayList

// 同步 handler：业务实现 (ctx, args) -> 结果。
public type Handler = (GuestCtx, JsonValue) -> JsonValue

public interface AbilityInstance {
    func manifest(): JsonValue
    func onLifecycle(phase: String, ctx: GuestCtx): Unit
    func invoke(method: String, ctx: GuestCtx, args: JsonValue): JsonValue
}

public interface AbilityProvider {
    func manifest(): JsonValue
    func create(config: JsonValue): AbilityInstance
}

public class AbilitySpec {
    public let id: String
    public var depends: ArrayList<JsonValue>
    public var events: ArrayList<JsonValue>
    // (methodId, description, inputSchema, handler)
    public var handlers: ArrayList<(String, String, JsonValue, Handler)>
    public var stateful: Bool
    public var configSchema: JsonValue

    public init(id: String) {
        this.id = id
        this.depends = ArrayList<JsonValue>()
        this.events = ArrayList<JsonValue>()
        this.handlers = ArrayList<(String, String, JsonValue, Handler)>()
        this.stateful = false
        this.configSchema = JNull
    }

    public static func must(id: String): AbilitySpec {
        return AbilitySpec(id)
    }

    public func dependsAlias(abilityId: String, alias: String): AbilitySpec {
        this.depends.add(jObjOf([
            ("id", JStr(abilityId)),
            ("alias", JStr(alias)),
            ("optional", JBool(false)),
        ]))
        return this
    }

    public func eventEmit(name: String, description: String, payloadSchema: JsonValue): AbilitySpec {
        this.events.add(jObjOf([
            ("name", JStr(name)),
            ("dir", JStr("emit")),
            ("description", JStr(description)),
            ("payload_schema", payloadSchema),
        ]))
        return this
    }

    public func on(methodId: String, description: String, inputSchema: JsonValue, handler: Handler): AbilitySpec {
        this.handlers.add((methodId, description, inputSchema, handler))
        return this
    }

    public func manifest(): JsonValue {
        let methods = ArrayList<JsonValue>()
        for (h in this.handlers) {
            methods.add(jObjOf([
                ("id", JStr(h[0])),
                ("description", JStr(h[1])),
                ("input_schema", h[2]),
                ("output_schema", JNull),
            ]))
        }
        return jObjOf([
            ("id", JStr(this.id)),
            ("depends", JArr(this.depends)),
            ("stateful", JBool(this.stateful)),
            ("config_schema", this.configSchema),
            ("methods", JArr(methods)),
            ("events", JArr(this.events)),
        ])
    }

    public func intoProvider(): AbilityProvider {
        return SpecProvider(this)
    }
}

// spec 直接落到 provider：invoke 分发到注册的 handler。
internal class SpecProvider <: AbilityProvider {
    let spec: AbilitySpec

    init(spec: AbilitySpec) {
        this.spec = spec
    }

    public func manifest(): JsonValue {
        return spec.manifest()
    }

    public func create(_config: JsonValue): AbilityInstance {
        return SpecInstance(spec)
    }
}

internal class SpecInstance <: AbilityInstance {
    let spec: AbilitySpec

    init(spec: AbilitySpec) {
        this.spec = spec
    }

    public func manifest(): JsonValue {
        return spec.manifest()
    }

    public func onLifecycle(_phase: String, _ctx: GuestCtx): Unit {
        return ()
    }

    public func invoke(method: String, ctx: GuestCtx, args: JsonValue): JsonValue {
        for (h in spec.handlers) {
            if (h[0] == method) {
                return h[3](ctx, args)
            }
        }
        throw SdkError("未知方法: ${method}", "not_found")
    }
}
