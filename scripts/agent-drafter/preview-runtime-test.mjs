#!/usr/bin/env node
/*
 * Verify authored scripts and handlers in a real daemon-served app, under its
 * actual CSP. Runtime HTML must not reuse an authored handler or binder marker.
 * Replaces the obsolete iframe bridge test: previews now load the served app.
 *
 * BIOROUTER_TEST_BIN=/path/to/biorouterd node scripts/agent-drafter/preview-runtime-test.mjs
 * PW_EXEC=/path/to/chromium overrides the default system Google Chrome.
 * BIOROUTER_PLAYWRIGHT_MODULE=/path/to/playwright/index.js selects a dependency
 * installation outside the current checkout. No model or credentials are used.
 */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { randomBytes } from "node:crypto";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const playwright = await import(
  pathToFileURL(
    process.env.BIOROUTER_PLAYWRIGHT_MODULE ||
      join(root, "ui/desktop/node_modules/playwright/index.js"),
  ).href
);
const chromium = playwright.chromium || playwright.default.chromium;
const daemonBin =
  process.env.BIOROUTER_TEST_BIN || join(root, "target/debug/biorouterd");
const sandbox = mkdtempSync(join(tmpdir(), "biorouter-app-policy-"));
process.once("exit", () => rmSync(sandbox, { recursive: true, force: true }));
const appDir = join(sandbox, "config/agent_drafter/inline-policy-test");
const secret = randomBytes(32).toString("hex");
const handler = "record(this.dataset.q); return false";
const html = `<!doctype html><html><head><title>Inline policy regression</title></head>
<body onload="document.body.dataset.loaded = 'yes'">
<script>window.__hits = []; window.__inlineRan = true;
function record(q) { window.__hits.push(q); document.getElementById('out').textContent = window.__hits.join(','); }
</script>
<button id="authored" data-q="authored" onclick="${handler}">Authored</button>
<a id="cancel" href="#unexpected" onclick="return false">Cancel navigation</a>
<form><input name="amount" value="7"><button id="scope" onclick="document.getElementById('scoped').textContent = amount.value; return false">Form scope</button></form>
<p id="out"></p><p id="scoped"></p><div id="sink"></div>
<template id="template"><button onclick="${handler}" data-q="template">Template</button></template>
</body></html>`;
mkdirSync(appDir, { recursive: true });
mkdirSync(join(sandbox, "home"), { recursive: true });
writeFileSync(
  join(appDir, "manifest.json"),
  JSON.stringify({
    id: "inline-policy-test",
    title: "Inline policy regression",
    kind: "static",
    entry: "index.html",
  }),
);
writeFileSync(join(appDir, "index.html"), html);

const portProbe = createServer();
portProbe.listen(0, "127.0.0.1");
await once(portProbe, "listening");
const port = portProbe.address().port;
await new Promise((done) => portProbe.close(done));
const origin = `http://127.0.0.1:${port}`;
const childEnv = {
  PATH: process.env.PATH,
  HOME: join(sandbox, "home"),
  USERPROFILE: join(sandbox, "home"),
  BIOROUTER_PATH_ROOT: sandbox,
  BIOROUTER_HOST: "127.0.0.1",
  BIOROUTER_PORT: String(port),
  BIOROUTER_SERVER__SECRET_KEY: secret,
};
for (const name of ["SystemRoot", "TMPDIR", "TEMP", "TMP", "LANG"]) {
  if (process.env[name]) childEnv[name] = process.env[name];
}
const daemon = spawn(daemonBin, ["agent"], {
  env: childEnv,
  cwd: sandbox,
  stdio: "ignore",
});
const exited = once(daemon, "exit").catch(() => undefined);
let spawnError;
daemon.on("error", (error) => {
  spawnError = error;
});
let browser;
let assertions = 0;
function check(label, actual, expected = true) {
  assert.deepEqual(actual, expected, label);
  assertions++;
  console.log(`[app-policy] PASS ${label}`);
}
const pause = (ms) => new Promise((done) => setTimeout(done, ms));

try {
  let ready = false;
  for (let attempt = 0; attempt < 100; attempt++) {
    if (spawnError) throw spawnError;
    if (daemon.exitCode !== null)
      throw new Error("test daemon exited before readiness");
    try {
      const status = await fetch(`${origin}/status`, {
        headers: { "x-secret-key": secret },
        signal: AbortSignal.timeout(1000),
      });
      if (status.ok) {
        ready = true;
        break;
      }
    } catch {
      /* daemon is still starting */
    }
    await pause(100);
  }
  assert.ok(ready, "test daemon became ready");
  const launched = await fetch(`${origin}/apps/inline-policy-test/launch`, {
    method: "POST",
    headers: { "x-secret-key": secret },
  });
  assert.equal(launched.status, 200, "launch route succeeded");
  const { path } = await launched.json();
  browser = await chromium.launch(
    process.env.PW_EXEC
      ? { executablePath: process.env.PW_EXEC, headless: true }
      : { channel: "chrome", headless: true },
  );
  const page = await browser.newPage();
  await page.addInitScript(() => {
    window.__violations = [];
    document.addEventListener("securitypolicyviolation", (event) => {
      window.__violations.push(event.effectiveDirective);
    });
  });
  await page.goto(new URL(path, origin).href);
  await page.waitForFunction(() => window.__inlineRan === true);
  await page.waitForFunction(() => document.body.dataset.loaded === "yes");
  const appUrl = `${origin}/apps/inline-policy-test/`;
  const response = await page.reload();
  await page.waitForFunction(() => window.__inlineRan === true);
  await page.waitForFunction(() => document.body.dataset.loaded === "yes");
  check("launched page uses its daemon origin", page.url(), appUrl);
  const csp = response.headers()["content-security-policy"];
  const scriptPolicy = /(?:^|;)\s*script-src\s+([^;]+)/.exec(csp)?.[1] || "";
  check(
    "actual response CSP hashes authored scripts",
    scriptPolicy.includes("'sha256-"),
  );
  check(
    "actual response CSP avoids handler hash gadgets",
    !scriptPolicy.includes("unsafe-hashes") &&
      !scriptPolicy.includes("unsafe-inline"),
  );
  check(
    "authored inline script runs",
    await page.evaluate(() => window.__inlineRan),
  );
  check(
    "body onload is bound to the window",
    await page.locator("body").getAttribute("data-loaded"),
    "yes",
  );
  await page.click("#authored");
  check(
    "authored onclick runs with the authored element as this",
    await page.locator("#out").textContent(),
    "authored",
  );
  await page.click("#cancel");
  check("return false cancels navigation", page.url(), appUrl);
  await page.click("#scope");
  check(
    "handler names resolve through their form",
    await page.locator("#scoped").textContent(),
    "7",
  );
  check(
    "binder removes its markers",
    await page.locator("[data-br-on-click]").count(),
    0,
  );

  await page.evaluate((code) => {
    const img = document.createElement("img");
    img.src = "assets/missing.png";
    img.dataset.q = "injected";
    img.setAttribute("onerror", code);
    document.getElementById("sink").append(img);
  }, handler);
  await page.waitForFunction(() =>
    window.__violations.includes("script-src-attr"),
  );
  check(
    "injected onerror cannot reuse the authored onclick body",
    await page.evaluate(() => window.__hits),
    ["authored"],
  );

  await page.evaluate(() => {
    const binder = document.getElementById("biorouter-inline-handlers");
    const nonce = /var nonce = "([^"]+)"/.exec(binder.textContent)[1];
    const late = document.createElement("button");
    late.id = "late";
    late.dataset.q = "late";
    late.setAttribute("data-br-on-click", `${nonce}.0`);
    document.getElementById("sink").append(late);
    const replay = document.createElement("script");
    replay.textContent = binder.textContent;
    document.body.append(replay);
    late.click();
  });
  check(
    "replaying the allowed binder does not bind runtime markers",
    await page.evaluate(() => window.__hits),
    ["authored"],
  );
  await page.evaluate(() => {
    const clone = document.getElementById("template").content.cloneNode(true);
    document.getElementById("sink").append(clone);
    document.querySelector('#sink button[data-q="template"]').click();
  });
  check(
    "a cloned template handler remains blocked",
    await page.evaluate(() => window.__hits),
    ["authored"],
  );
  console.log(`[app-policy] ${assertions} browser assertions passed`);
} finally {
  await browser?.close();
  if (daemon.exitCode === null && !spawnError) {
    daemon.kill("SIGTERM");
    await Promise.race([exited, pause(5000)]);
    if (daemon.exitCode === null) {
      daemon.kill("SIGKILL");
      await exited;
    }
  }
  rmSync(sandbox, { recursive: true, force: true });
}
