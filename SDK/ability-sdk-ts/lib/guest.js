import { createInterface } from "node:readline";
import { stdin as input, stdout as output } from "node:process";
import {
  decodeFrame,
  encodeFrame,
  frameReply,
  okValue,
} from "./protocol.js";
import { submitFeedback } from "./feedback.js";

function wireError(err) {
  const kind = err.kind ?? "failed";
  return { type: "err", error: { kind, message: String(err.message ?? err) } };
}

function fail(message, kind = "failed") {
  return Object.assign(new Error(message), { kind });
}

function createBridge(writeLine) {
  const pending = new Map();
  const subs = new Map();
  const handlers = new Map();
  let guestIdSeq = 1;

  async function send(frame) {
    writeLine(encodeFrame(frame));
  }

  async function request(body) {
    const frame = { id: crypto.randomUUID(), body };
    return new Promise((resolve, reject) => {
      pending.set(frame.id, { resolve, reject });
      send(frame).catch(reject);
    });
  }

  function dispatchIncoming(frame) {
    if (frame.reply_to && pending.has(frame.reply_to)) {
      const { resolve } = pending.get(frame.reply_to);
      pending.delete(frame.reply_to);
      resolve(frame);
      return true;
    }
    return false;
  }

  function createGuestCtx(ctxId, instance, method, chainId, span, undos) {
    const ctx = {
      instance,
      method,
      chainId,
      span,
      async ctxOp(op) {
        const frame = await request({ type: "ctx", ctx_id: ctxId, op });
        return okValue(frame);
      },
      async share() {
        try {
          const v = await ctx.ctxOp({ op: "share" });
          const id = v && typeof v.ctx_id === "string" ? v.ctx_id : ctxId;
          return createGuestCtx(id, instance, null, chainId, span, []);
        } catch {
          return createGuestCtx(ctxId, instance, null, chainId, span, []);
        }
      },
      retain() {
        return createGuestCtx(ctxId, instance, method, chainId, span, undos);
      },
      chainStatus(chainIdOpt) {
        return ctx
          .ctxOp({ op: "chain_status", chain_id: chainIdOpt ?? null })
          .catch(() => null);
      },
      async call(idOrAlias, methodName, args) {
        return ctx.ctxOp({
          op: "call",
          id_or_alias: idOrAlias,
          method: methodName,
          args,
        });
      },
      async dep(idOrAlias) {
        const v = await ctx.ctxOp({ op: "dep", id_or_alias: idOrAlias });
        if (typeof v !== "string") {
          throw new Error("dep 返回非字符串");
        }
        return v;
      },
      async catalog() {
        try {
          return await ctx.ctxOp({ op: "catalog" });
        } catch {
          return [];
        }
      },
      async localGet(key) {
        const v = await ctx.ctxOp({ op: "local_get", key });
        if (v?.hit) {
          return v.value;
        }
        return undefined;
      },
      async localSet(key, value) {
        await ctx.ctxOp({ op: "local_set", key, value });
      },
      async emit(name, payload) {
        await ctx.ctxOp({ op: "emit", name, payload });
      },
      async load(spec) {
        const v = await ctx.ctxOp({ op: "load", spec });
        if (!v || typeof v.instance !== "string") {
          throw fail("load 未返回 instance");
        }
        return v.instance;
      },
      async unload(instance) {
        await ctx.ctxOp({ op: "unload", instance });
      },
      async subscribe(filter) {
        const v = await ctx.ctxOp({ op: "subscribe", filter });
        if (!v || typeof v.sub !== "string" || !v.sub) {
          throw fail("Subscribe 未返回 sub");
        }
        const sub = v.sub;
        const queue = [];
        const waiters = [];
        subs.set(sub, (event) => {
          if (waiters.length) {
            waiters.shift()(event);
          } else {
            queue.push(event);
          }
        });
        return {
          async next() {
            if (queue.length) {
              return queue.shift();
            }
            return new Promise((r) => waiters.push(r));
          },
        };
      },
      async handle(filter, handler, opts = {}) {
        const guestId = `h${guestIdSeq++}`;
        handlers.set(guestId, { handler, ctxId });
        const v = await ctx.ctxOp({
          op: "handle",
          filter,
          prepend: !!opts.prepend,
          mode: handler.mode ?? "message",
          guest_id: guestId,
        });
        if (!v || !Number.isInteger(v.id)) {
          throw fail("Handle 未返回 id");
        }
        const regId = v.id;
        return {
          unregister() {
            handlers.delete(guestId);
            request({
              type: "ctx",
              ctx_id: ctxId,
              op: { op: "unhandle", id: regId },
            }).catch(() => {});
          },
        };
      },
      async pushEffect(effect) {
        const undo = await effect.apply(ctx);
        undos.push(undo);
      },
      async feedback(tags, desc) {
        await submitFeedback(ctx, tags, desc);
      },
      registerAspect(_aspect) {
        throw fail("Guest 不支持 register_aspect");
      },
      withProtocolFilter(_kind) {
        return ctx.retain();
      },
    };
    return ctx;
  }

  async function runGuestHandle(guestId, event, nextToken) {
    const slot = handlers.get(guestId);
    if (!slot) {
      throw Object.assign(new Error("事件处理者已卸"), { kind: "not_found" });
    }
    const h = slot.handler;
    const mode = h.mode ?? "message";
    if (mode === "middleware") {
      if (!h.around) {
        throw fail("middleware 缺少 around");
      }
      const next = async (ev) => {
        if (!nextToken) {
          return ev;
        }
        const v = await request({
          type: "ctx",
          ctx_id: slot.ctxId,
          op: { op: "event_next", token: nextToken, event: ev },
        });
        return okValue(v);
      };
      return h.around(event, next);
    }
    if (mode === "pipeline") {
      if (!h.process) {
        throw fail("pipeline 缺少 process");
      }
      return h.process(event);
    }
    if (mode === "message" || mode === "parallel") {
      if (!h.onEvent) {
        throw fail(`${mode} 缺少 onEvent`);
      }
      await h.onEvent(event);
      return event;
    }
    throw fail(`未知事件模式: ${mode}`);
  }

  return {
    send,
    request,
    dispatchIncoming,
    subs,
    handlers,
    createGuestCtx,
    runGuestHandle,
  };
}

async function runUndos(undos, origin) {
  let acc = origin;
  while (undos.length) {
    const u = undos.pop();
    try {
      if (u?.run) {
        await u.run();
      }
    } catch (e) {
      if (!(acc instanceof Error)) {
        acc = fail(`Undo 失败: ${e.message ?? e}`);
      }
    }
  }
  if (acc instanceof Error) {
    throw acc;
  }
  return acc;
}

/**
 * @param {import('./spec.js').AbilityProvider} provider
 */
export async function serveStdio(provider) {
  const bridge = createBridge((line) => output.write(line));
  let abilitySlot = null;
  const rl = createInterface({ input, crlfDelay: Infinity });
  // 命令帧顺序执行。读循环不等待命令，应答与 run_handle 可以重入。
  let commandTail = Promise.resolve();
  function runCommand(task) {
    const run = commandTail.then(task, task);
    commandTail = run.then(
      () => {},
      () => {},
    );
  }

  for await (const line of rl) {
    if (!line.trim()) {
      continue;
    }
    const frame = decodeFrame(line);
    if (bridge.dispatchIncoming(frame)) {
      continue;
    }
    const body = frame.body;
    switch (body.type) {
      case "event": {
        const tx = bridge.subs.get(body.sub);
        if (tx) {
          tx(body.event);
        }
        break;
      }
      case "run_handle": {
        const cmdId = frame.id;
        const { guest_id, event, next_token } = body;
        (async () => {
          let reply;
          try {
            const value = await bridge.runGuestHandle(
              guest_id,
              event,
              next_token ?? null,
            );
            reply = { type: "ok", value };
          } catch (e) {
            reply = wireError(e);
          }
          await bridge.send(frameReply(cmdId, reply));
        })();
        break;
      }
      case "shutdown":
        return;
      case "hello":
        await bridge.send(
          frameReply(frame.id, {
            type: "manifest",
            manifest: provider.manifest(),
          }),
        );
        break;
      case "create": {
        try {
          abilitySlot = await provider.create(body.config);
          await bridge.send(frameReply(frame.id, { type: "ok", value: null }));
        } catch (e) {
          await bridge.send(frameReply(frame.id, wireError(e)));
        }
        break;
      }
      case "invoke": {
        const cmdId = frame.id;
        const ability = abilitySlot;
        runCommand(async () => {
          let reply;
          if (!ability) {
            reply = {
              type: "err",
              error: { kind: "failed", message: "尚未 Create" },
            };
          } else {
            const undos = [];
            const ctx = bridge.createGuestCtx(
              cmdId,
              body.instance,
              body.method,
              body.chain_id,
              body.span,
              undos,
            );
            try {
              const result = await ability.invoke(body.method, ctx, body.args);
              const value = await runUndos(undos, result);
              reply = { type: "ok", value };
            } catch (e) {
              try {
                await runUndos(undos, e);
              } catch (ue) {
                reply = wireError(ue);
              }
              if (!reply) {
                reply = wireError(e);
              }
            }
          }
          await bridge.send(frameReply(cmdId, reply));
        });
        break;
      }
      case "lifecycle": {
        const cmdId = frame.id;
        const ability = abilitySlot;
        runCommand(async () => {
          let reply;
          if (!ability) {
            reply = {
              type: "err",
              error: { kind: "failed", message: "尚未 Create" },
            };
          } else {
            const undos = [];
            const ctx = bridge.createGuestCtx(
              cmdId,
              body.instance,
              null,
              body.chain_id,
              body.span,
              undos,
            );
            try {
              await ability.onLifecycle(body.phase, ctx);
              reply = { type: "ok", value: null };
            } catch (e) {
              reply = wireError(e);
            }
          }
          await bridge.send(frameReply(cmdId, reply));
        });
        break;
      }
      default:
        break;
    }
  }
}
