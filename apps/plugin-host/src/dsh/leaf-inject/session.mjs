const LEAF_SESSION_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * 官方 SessionStore 已由 services 挂到 `ctx.sessions`。
 * 会话 id 为 Leaf ConversationId 时，把 user/assistant 消息投影到 `session.*` 出站。
 * 不把出站追加再写回 store，避免来回复制。
 */
export function registerSessionInject(ctx, hostCall) {
  if (!hostCall) return;
  ctx.on("session/event", (session, event) => {
    const id = session?.id;
    if (typeof id !== "string" || !LEAF_SESSION_ID.test(id)) return;
    const text = messageText(event?.data);
    if (!text) return;
    const method = event?.type === "user/message"
      ? "append_user"
      : event?.type === "assistant/message"
        ? "append_assistant"
        : null;
    if (!method) return;
    void hostCall({
      ns: "session",
      method,
      args: [{ id, text }],
    }).catch((error) => {
      console.warn(`[leaf-dsh] session ${method} 失败: ${error?.message ?? error}`);
    });
  });
}

function messageText(message) {
  const content = message?.content;
  if (!Array.isArray(content)) return "";
  return content
    .filter((block) => block?.type === "text" && typeof block.text === "string")
    .map((block) => block.text)
    .join("\n");
}
