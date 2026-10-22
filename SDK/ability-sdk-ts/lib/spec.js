const FRAMEWORK_VERSION = "1";

export class AbilitySpec {
  constructor(id) {
    this.id = id;
    this.depends = [];
    this.events = [];
    this.handlers = new Map();
    this.stateful = false;
    this.configSchema = null;
  }

  static must(id) {
    return new AbilitySpec(id);
  }

  dependsAlias(abilityId, alias) {
    this.depends.push({ id: abilityId, alias, optional: false });
    return this;
  }

  eventEmit(name, description = "", payloadSchema = { type: "object" }) {
    this.events.push({
      name,
      dir: "emit",
      description,
      payload_schema: payloadSchema,
    });
    return this;
  }

  on(methodId, description, inputSchema, handler) {
    this.handlers.set(methodId, { description, inputSchema, handler });
    return this;
  }

  manifest() {
    return {
      id: this.id,
      depends: this.depends.map((d) => ({
        id: d.id,
        alias: d.alias ?? null,
        optional: d.optional ?? false,
      })),
      stateful: this.stateful,
      config_schema: this.configSchema ?? null,
      methods: [...this.handlers.entries()].map(([id, h]) => ({
        id,
        description: h.description,
        input_schema: h.inputSchema,
        output_schema: null,
      })),
      events: this.events,
    };
  }

  intoProvider() {
    const spec = this;
    return {
      manifest() {
        return spec.manifest();
      },
      async create(_config) {
        return {
          manifest() {
            return spec.manifest();
          },
          async onLifecycle(_phase, _ctx) {},
          async invoke(method, ctx, args) {
            const h = spec.handlers.get(method);
            if (!h) {
              throw Object.assign(new Error(`未知方法: ${method}`), {
                kind: "not_found",
              });
            }
            return h.handler(ctx, args);
          },
        };
      },
    };
  }
}

export { FRAMEWORK_VERSION };
