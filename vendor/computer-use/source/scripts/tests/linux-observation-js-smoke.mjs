#!/usr/bin/env node
// Runs inside the private GTK smoke session; Python owns fixture lifecycle.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JsonLinePeer, WorkerJavaScriptSession } from "../node-repl/open-computer-use-repl.mjs";

const [nativePath, appName, stateFile] = process.argv.slice(2);
assert.ok(nativePath && appName && stateFile, "Run through --observation-checks");
const native = new JsonLinePeer({ command: nativePath, args: ["mcp"],
  env: { ...process.env, OPEN_COMPUTER_USE_ACTION_READ_BACK: "0" } });
let session;
async function run(code, expectedError) {
  const result = await session.run(code);
  const text = result.content.filter(item => item.type === "text").map(item => item.text).join("\n");
  if (expectedError) {
    assert.equal(result.isError, true, text);
    assert.match(text, expectedError);
  } else assert.equal(result.isError, false, text);
  return result;
}
try {
  await native.initialize();
  const request = native.request.bind(native);
  native.request = async (method, params, timeout) => {
    const result = await request(method, params, timeout);
    if (method === "tools/call" && params.name === "get_app_state" && !result.isError) {
      const images = result.content.filter(item => item.type === "image");
      assert.equal(images.length, params.arguments.include_screenshot === false ? 0 : 1);
    }
    return result;
  };
  session = new WorkerJavaScriptSession({ native });
  const ax = await run(`var app = await cua.getApp(${JSON.stringify(appName)}, {initialState:false});
    var ax = await app.getAXState({includeScreenshot:false});
    var oldIndex = ax.match(/^\\s*(\\d+)\\s+[^\\n]*Read-back input(?:\\s|$)/m)[1];`);
  assert.equal(ax.content.some(item => item.type === "image"), false);
  const shot = await run(`var screenshot = await app.getScreenshot({includeAX:false});
    if(screenshot.subarray(0,8).toString('hex') !== '89504e470d0a1a0a') throw Error('Invalid PNG');`);
  assert.equal(shot.content.filter(item => item.type === "image").length, 1);
  await run(`await app.setValue(oldIndex, "must-not-write");`, /unknown element_index/);
  await run(`var fresh = await app.getAXState({includeScreenshot:false,emit:false});
    var index = fresh.match(/^\\s*(\\d+)\\s+[^\\n]*Read-back input(?:\\s|$)/m)[1];
    await app.setValue(index, "worker-live-中文🙂");`);
  const deadline = Date.now() + 5000;
  while (JSON.parse(readFileSync(stateFile, "utf8")).text !== "worker-live-中文🙂") {
    assert.ok(Date.now() < deadline, "GTK value did not update");
    await new Promise(resolve => setTimeout(resolve, 30));
  }
  const full = await run(`await app.getAXStateAndScreenshot();`);
  assert.equal(full.content.filter(item => item.type === "image").length, 1);
  assert.ok(full.content.some(item => item.type === "text" && item.text.includes("worker-live-中文🙂")));
  console.log("Worker JS → native MCP → GTK: selective collection, PNG output, stale-index error and AX recovery passed.");
} finally {
  await session?.close();
  native.close();
}
