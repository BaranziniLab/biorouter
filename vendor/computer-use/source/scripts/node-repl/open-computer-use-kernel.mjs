import { parentPort } from "node:worker_threads";
import { PersistentJavaScriptSession } from "./open-computer-use-repl.mjs";

let nextNativeId = 1;
let activeTimeoutMs;
const pendingNative = new Map();
const native = {
  request(method, params, timeoutMs = activeTimeoutMs) {
    const id = nextNativeId++;
    return new Promise((resolve, reject) => {
      pendingNative.set(id, { resolve, reject });
      parentPort.postMessage({ type: "native_request", id, method, params, timeoutMs });
    });
  },
};
const session = new PersistentJavaScriptSession({ native });

parentPort.on("message", async message => {
  if (message.type === "native_response") {
    const pending = pendingNative.get(message.id);
    if (!pending) return;
    pendingNative.delete(message.id);
    if (message.error) pending.reject(new Error(message.error));
    else pending.resolve(message.result);
    return;
  }
  if (message.type === "exec") {
    let result;
    try {
      activeTimeoutMs = message.timeoutMs;
      result = await session.run(message.code, 0, message.requestMeta);
    } catch (error) {
      result = { content: [{ type: "text", text: `Error: ${error instanceof Error ? error.message : String(error)}` }], isError: true };
    }
    activeTimeoutMs = undefined;
    parentPort.postMessage({ type: "result", id: message.id, result });
  }
});
