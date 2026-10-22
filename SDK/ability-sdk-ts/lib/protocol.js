/** @typedef {{ id: string, reply_to?: string, body: object }} Frame */

export const PROTOCOL_VERSION = "1";

export function encodeFrame(frame) {
  return JSON.stringify(frame) + "\n";
}

export function decodeFrame(line) {
  const trimmed = line.replace(/\r?\n$/, "").trim();
  if (!trimmed) {
    throw new Error("空帧");
  }
  return JSON.parse(trimmed);
}

export function frameRequest(body) {
  return {
    id: crypto.randomUUID(),
    body,
  };
}

export function frameReply(to, body) {
  return {
    id: crypto.randomUUID(),
    reply_to: to,
    body,
  };
}

export function okValue(frame) {
  const body = frame.body;
  if (body.type === "ok") {
    return body.value ?? null;
  }
  if (body.type === "err") {
    const e = body.error;
    throw Object.assign(new Error(e.message), { kind: e.kind });
  }
  throw new Error(`远程应答类型不符: ${JSON.stringify(body)}`);
}
