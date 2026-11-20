#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { JsonLinePeer, WorkerJavaScriptSession } from "../node-repl/open-computer-use-repl.mjs";

assert.equal(process.platform, "darwin");
assert.ok(process.argv[2], "Pass the normally signed OCU app executable with existing permissions/admission");
const nativePath = path.resolve(process.argv[2]);
const directory = mkdtempSync(path.join(tmpdir(), "ocu-observation-"));
const fixturePath = path.join(directory, "OCUObservationFixture");
const stateFile = path.join(directory, "state.json");
const appName = "OCUObservationFixture";
const env = { ...process.env, OPEN_COMPUTER_USE_DISABLE_APP_AGENT_PROXY: "1",
  OPEN_COMPUTER_USE_VISUAL_CURSOR: "0", OPEN_COMPUTER_USE_ACTION_READ_BACK: "0",
  OPEN_COMPUTER_USE_DEBUG_TIMING: "1", OPEN_COMPUTER_USE_CONFIG_FILE: path.join(directory, "config.json") };
for (const key of Object.keys(env)) if (key.startsWith("OPEN_COMPUTER_USE_IMAGE_")) delete env[key];
writeFileSync(env.OPEN_COMPUTER_USE_CONFIG_FILE, "{}");
let fixture, native, session;
const text = result => result.content.filter(item => item.type === "text").map(item => item.text).join("\n");
const images = result => result.content.filter(item => item.type === "image");
function indexOf(state) {
  const match = state.match(/^\s*(\d+)\s+[^\n]*Read-back input(?:[\s,]|$)/m);
  assert.ok(match, state);
  return match[1];
}
async function waitFor(test) {
  const deadline = Date.now() + 8000;
  while (!test()) { assert.ok(Date.now() < deadline, "Fixture timeout"); await new Promise(r => setTimeout(r, 30)); }
}
async function call(name, args, expectedError) {
  const result = await native.request("tools/call", { name, arguments: { app: appName, ...args } });
  assert.equal(!!result.isError, !!expectedError, text(result));
  if (expectedError) assert.match(text(result), expectedError);
  return result;
}
async function observe(args) {
  native.stderr = "";
  const result = await call("get_app_state", args);
  await new Promise(r => setTimeout(r, 50));
  if (args.include_screenshot === false) {
    assert.equal(images(result).length, 0);
    assert.doesNotMatch(native.stderr, /timing snapshot\.capture_(?:hw|sck) /);
    assert.match(native.stderr, /timing snapshot\.tree_walk /);
  } else {
    assert.equal(images(result).length, 1, text(result));
    assert.equal(Buffer.from(images(result)[0].data, "base64").subarray(0, 8).toString("hex"), "89504e470d0a1a0a");
    assert.match(native.stderr, /timing snapshot\.capture_(?:hw|sck) /);
  }
  if (args.include_ax === false) {
    assert.doesNotMatch(native.stderr, /timing snapshot\.tree_walk /);
    assert.ok(!text(result).includes("Read-back input"));
    assert.match(text(result), /indexes invalidated/);
  }
  return result;
}
try {
  const source = fileURLToPath(new URL("./fixtures/MacOSObservationFixture.swift", import.meta.url));
  const build = spawnSync("swiftc", [source, "-o", fixturePath], { encoding: "utf8" });
  assert.equal(build.status, 0, build.stderr);
  fixture = spawn(fixturePath, [stateFile], { stdio: ["ignore", "ignore", "ignore"] });
  await waitFor(() => existsSync(stateFile));
  native = new JsonLinePeer({ command: nativePath, args: ["mcp"], env });
  await native.initialize();
  const full = await observe({});
  const oldIndex = indexOf(text(full));
  const queried = JSON.parse(text(await call("query", { role: "AXTextField" })));
  assert.equal(queried.matches.length, 1);
  await observe({ include_ax: false });
  await call("set_value", { element_index: oldIndex, value: "bad" }, /unknown element_index/);
  await call("set_value", { element_index: queried.matches[0].index, value: "bad" }, /expired|no longer available/);
  const ax = await observe({ include_screenshot: false });
  await call("click", { x: 10, y: 10 }, /include_screenshot=false/);
  await call("set_value", { element_index: indexOf(text(ax)), value: "native-live-中文🙂" });
  await waitFor(() => JSON.parse(readFileSync(stateFile, "utf8")).text === "native-live-中文🙂");
  for (const command of [["snapshot", "--include-screenshot", "false", appName],
    ["call", "get_app_state", "--args", JSON.stringify({ app: appName, include_screenshot: false })]]) {
    const cli = spawnSync(nativePath, command, { env, encoding: "utf8", timeout: 15000 });
    assert.equal(cli.status, 0, cli.stderr);
    assert.match(cli.stdout, /include_screenshot=false/);
    if (command[0] === "call") assert.equal(images(JSON.parse(cli.stdout)).length, 0);
  }
  session = new WorkerJavaScriptSession({ native });
  let result = await session.run(`var app = await cua.getApp(${JSON.stringify(appName)}, {initialState:false});
    var state = await app.getAXState({includeScreenshot:false});`);
  assert.equal(result.isError, false, text(result));
  assert.equal(images(result).length, 0);
  result = await session.run("await app.getScreenshot({includeAX:false});");
  assert.equal(result.isError, false, text(result));
  assert.equal(images(result).length, 1);
  result = await session.run(`var fresh = await app.getAXState({includeScreenshot:false,emit:false});
    var index = fresh.match(/^\\s*(\\d+)\\s+[^\\n]*Read-back input(?:[\\s,]|$)/m)[1];
    await app.setValue(index, "worker-live-中文🙂");`);
  assert.equal(result.isError, false, text(result));
  await waitFor(() => JSON.parse(readFileSync(stateFile, "utf8")).text === "worker-live-中文🙂");
  console.log("macOS real AX/native MCP/CLI/Worker JS: collection gates, PNG, snapshot/query invalidation, coordinate refusal and element recovery passed.");
} finally {
  await session?.close();
  native?.close();
  if (fixture) {
    fixture.kill("SIGTERM");
    if (fixture.exitCode === null && fixture.signalCode === null) await new Promise(resolve => {
      const timer = setTimeout(() => { fixture.kill("SIGKILL"); resolve(); }, 3000);
      fixture.once("exit", () => { clearTimeout(timer); resolve(); });
    });
  }
  rmSync(directory, { recursive: true, force: true });
}
