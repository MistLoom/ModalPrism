import { randomUUID } from "node:crypto";

/** Guest 向宿主提交反馈（对齐 `CtxOp::Feedback`）。 */
export function submitFeedback(ctx, tags, desc) {
  const tagList = Array.isArray(tags) ? tags : [tags];
  return ctx.ctxOp({
    op: "feedback",
    record: {
      id: randomUUID(),
      at: Date.now(),
      loc: {
        ability: ctx.instance ?? "guest",
        module: "serve",
      },
      tags: tagList.map(String),
      desc: String(desc),
      chain_id: ctx.chainId ?? "detached",
      span: ctx.span ?? 0,
    },
  });
}
