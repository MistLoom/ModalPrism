#!/usr/bin/env node
import { register } from "node:module";
import { createInterface } from "node:readline";

await register("./src/host-resolve.mjs", import.meta.url);
import { HostRuntime } from "./src/runtime.mjs";

const runtime = new HostRuntime((frame) => {
  process.stdout.write(JSON.stringify(frame) + "\n");
});

const rl = createInterface({ input: process.stdin, terminal: false });
for await (const line of rl) {
  const trimmed = line.trim();
  if (!trimmed) continue;
  let msg;
  try {
    msg = JSON.parse(trimmed);
  } catch {
    runtime.send({ type: "error", message: "invalid_json" });
    continue;
  }
  await runtime.handle(msg);
}
