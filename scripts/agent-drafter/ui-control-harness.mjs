/**
 * Deterministic harness for the agent-driven UI runtime.
 *
 * Serves a built Agent Drafter app and stands in for `biorouterd`'s per-app
 * agent socket, speaking the real wire protocol. That lets us exercise every
 * `ui` command (and the `ui_ask` round-trip) against the real `sdk.ts` in a real
 * browser, with none of an LLM's nondeterminism.
 *
 *   node scripts/agent-drafter/ui-control-harness.mjs --app <dir> [--port 8899]
 *
 * The page drives it with `window.__harness.send({cmd:"panel", ...})`, which
 * POSTs to /__emit; the harness relays it down the socket as a `ui` frame.
 * Anything the browser sends back (`ui_reply`, `ui_surface`, `widget_action`) is
 * recorded and readable at GET /__frames.
 *
 * Zero dependencies: the WebSocket server is ~60 lines of RFC 6455.
 */
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { extname, resolve, normalize, join, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const argOf = (name, dflt) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : dflt;
};
const APP = resolve(argOf("--app", "."));
const PORT = Number(argOf("--port", 8899));
// With no `--app`, the harness runs its own SDK v2 scenarios (below): it bundles
// the real sdk.ts, mounts it in jsdom against this mock daemon, drives the state
// channel + bindings, and asserts. With `--app`, it stays a plain server a real
// browser drives via /__emit (the original behavior).
const SELFTEST = args.indexOf("--app") < 0;
/** The upgrade request URL of the most recent socket (for the wsToken check). */
let lastUpgradeUrl = null;

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json",
};

/** Frames the browser sent us, in order. */
const received = [];
/** The live client socket, if any. */
let client = null;

// ── RFC 6455: just enough to send/receive unfragmented text frames ──────────

const GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

function encodeFrame(text) {
  const payload = Buffer.from(text, "utf8");
  const len = payload.length;
  let header;
  if (len < 126) {
    header = Buffer.alloc(2);
    header[1] = len;
  } else if (len < 65536) {
    header = Buffer.alloc(4);
    header[1] = 126;
    header.writeUInt16BE(len, 2);
  } else {
    header = Buffer.alloc(10);
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(len), 2);
  }
  header[0] = 0x81; // FIN + text
  return Buffer.concat([header, payload]);
}

/** Pull every complete text frame out of `buf`; returns [texts, rest]. */
function decodeFrames(buf) {
  const out = [];
  let i = 0;
  while (i + 2 <= buf.length) {
    const opcode = buf[i] & 0x0f;
    const masked = (buf[i + 1] & 0x80) !== 0;
    let len = buf[i + 1] & 0x7f;
    let off = i + 2;
    if (len === 126) {
      if (off + 2 > buf.length) break;
      len = buf.readUInt16BE(off);
      off += 2;
    } else if (len === 127) {
      if (off + 8 > buf.length) break;
      len = Number(buf.readBigUInt64BE(off));
      off += 8;
    }
    let mask = null;
    if (masked) {
      if (off + 4 > buf.length) break;
      mask = buf.subarray(off, off + 4);
      off += 4;
    }
    if (off + len > buf.length) break;
    const payload = Buffer.from(buf.subarray(off, off + len));
    if (mask) for (let k = 0; k < payload.length; k++) payload[k] ^= mask[k % 4];
    if (opcode === 0x1) out.push(payload.toString("utf8"));
    if (opcode === 0x8) out.push(null); // close
    i = off + len;
  }
  return [out, buf.subarray(i)];
}

const send = (obj) => client && client.write(encodeFrame(JSON.stringify(obj)));

// ── HTTP: static files + the harness control plane ──────────────────────────

const server = createServer(async (req, res) => {
  const url = (req.url || "/").split("?")[0];

  if (url === "/__frames") {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify(received));
    return;
  }
  if (url === "/__emit" && req.method === "POST") {
    let body = "";
    for await (const chunk of req) body += chunk;
    const cmd = JSON.parse(body);
    send({ type: "ui", ...cmd });
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ ok: !!client }));
    return;
  }
  if (url === "/__reset" && req.method === "POST") {
    received.length = 0;
    res.writeHead(200);
    res.end("ok");
    return;
  }
  // The SDK's model surface; keeps `listModels()` from erroring in the console.
  // (It carries the page token as `?token=`, as the daemon requires.)
  if (url.split("?")[0].endsWith("/models")) {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ providers: [] }));
    return;
  }

  let path = decodeURIComponent(url);
  if (path === "/") path = "/index.html";
  const file = resolve(APP, "." + normalize(path));
  if (file !== APP && !file.startsWith(APP + "/")) {
    res.writeHead(403);
    res.end("Forbidden");
    return;
  }
  try {
    const body = await readFile(file);
    res.writeHead(200, { "Content-Type": MIME[extname(file)] || "application/octet-stream" });
    res.end(body);
  } catch {
    res.writeHead(404);
    res.end("Not found");
  }
});

// ── The per-app agent socket ────────────────────────────────────────────────

server.on("upgrade", (req, socket) => {
  lastUpgradeUrl = req.url || null;
  const key = req.headers["sec-websocket-key"];
  const accept = createHash("sha1").update(key + GUID).digest("base64");
  socket.write(
    "HTTP/1.1 101 Switching Protocols\r\n" +
      "Upgrade: websocket\r\nConnection: Upgrade\r\n" +
      `Sec-WebSocket-Accept: ${accept}\r\n\r\n`
  );
  client = socket;

  let buf = Buffer.alloc(0);
  socket.on("data", (chunk) => {
    buf = Buffer.concat([buf, chunk]);
    const [frames, rest] = decodeFrames(buf);
    buf = rest;
    for (const text of frames) {
      if (text === null) {
        socket.destroy();
        return;
      }
      try {
        received.push(JSON.parse(text));
      } catch {
        received.push({ raw: text });
      }
    }
  });
  socket.on("close", () => {
    if (client === socket) client = null;
  });
  socket.on("error", () => {});

  // Advertise `ui` so `sdk.ts` mounts the runtime and reports its surface.
  send({
    type: "ready",
    protocol: 2,
    capabilities: ["ui"],
    sessionId: "harness",
    resumed: false,
    messageCount: 0,
  });
});

server.listen(PORT, "127.0.0.1", () => {
  if (SELFTEST) {
    runSelftest()
      .then((code) => process.exit(code))
      .catch((e) => {
        console.error("[ui-harness] ERROR", e && e.stack ? e.stack : e);
        process.exit(2);
      });
    return;
  }
  console.log(`ui-control harness: app=${APP} on http://127.0.0.1:${PORT}`);
});

// ── SDK v2 self-test: real sdk.ts in jsdom against this mock daemon ──────────
// Bundles the shipped sdk.ts (via the real app entry `main.ts`, exactly like
// `build_app`), mounts it in jsdom with Node's global WebSocket pointed at this
// server, then drives the shared-state channel, declarative bindings, frame
// tolerance, and wsToken — asserting on the DOM and on the frames the SDK sends
// back. Exits non-zero on any failure, matching the sibling test kits.

const log = (...a) => console.log("[ui-harness]", ...a);
let failures = 0;
function check(name, cond, detail) {
  if (cond) log(`PASS — ${name}`);
  else {
    failures++;
    console.error(`[ui-harness] FAIL — ${name}${detail ? ": " + detail : ""}`);
  }
}
const delay = (ms) => new Promise((r) => setTimeout(r, ms));
async function waitFor(pred, ms = 4000, step = 15) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    let ok = false;
    try {
      ok = pred();
    } catch {
      ok = false;
    }
    if (ok) return true;
    await delay(step);
  }
  return false;
}

function firstExisting(paths) {
  for (const p of paths) if (p && existsSync(p)) return p;
  return null;
}

/** Emit a server→client `ui` frame down the socket (as `control.rs::emit` does,
 *  now stamped with `"v": 1`). */
function emitUi(frame) {
  send({ type: "ui", v: 1, ...frame });
}

async function runSelftest() {
  const WS_TOKEN = "tok-abc123";
  const APP_ID = "harness";

  // 1) Resolve the real esbuild + jsdom (worktrees have no node_modules, so a
  //    full checkout or the BIOROUTER_ESBUILD_BIN / BIOROUTER_JSDOM_DIR env
  //    vars point us at one).
  const esbuild =
    process.env.BIOROUTER_ESBUILD_BIN ||
    firstExisting([join(HERE, "../../ui/desktop/node_modules/.bin/esbuild")]);
  const jsdomDir =
    process.env.BIOROUTER_JSDOM_DIR ||
    firstExisting([join(HERE, "../../ui/desktop/node_modules/jsdom")]);
  if (!esbuild || !jsdomDir) {
    console.error(
      "[ui-harness] need esbuild + jsdom. Set BIOROUTER_ESBUILD_BIN and " +
        "BIOROUTER_JSDOM_DIR, or run from a checkout with ui/desktop/node_modules."
    );
    return 2;
  }

  // 2) Bundle the shipped SDK through the real app entry (main.ts imports ./sdk).
  const mainTs = join(
    HERE,
    "../../crates/biorouter-mcp/src/agent_drafter/templates/main.ts"
  );
  let bundle;
  try {
    bundle = execFileSync(
      esbuild,
      [mainTs, "--bundle", "--format=iife", "--target=es2018", "--loader:.ts=ts", "--log-level=warning"],
      { encoding: "utf8", maxBuffer: 32 * 1024 * 1024 }
    );
  } catch (e) {
    console.error("[ui-harness] esbuild failed:", e.stderr || e.message);
    return 2;
  }

  // 3) Mount the bundle in jsdom with a real WebSocket to this daemon.
  const { JSDOM } = await import(pathToFileURL(join(jsdomDir, "lib/api.js")).href);
  const html =
    "<!doctype html><html><head><title>Harness</title></head><body>" +
    '<section data-br-region="results"></section>' +
    '<span id="cnt" data-br-bind="/cohort/count">-</span>' +
    '<a id="lnk" data-br-bind-attr="href:/link">link</a>' +
    '<a id="jsl" data-br-bind-attr="href:/danger">danger</a>' +
    '<button id="clk" data-br-bind-attr="onclick:/handler">btn</button>' +
    '<div id="box" data-br-bind-show="/visible">boxed</div>' +
    "</body></html>";
  const dom = new JSDOM(html, {
    url: `http://127.0.0.1:${PORT}/`,
    runScripts: "outside-only",
    pretendToBeVisual: true,
  });
  const win = dom.window;
  const warnLog = [];
  win.WebSocket = globalThis.WebSocket;
  win.console = {
    log: () => {},
    info: () => {},
    debug: () => {},
    error: () => {},
    warn: (...a) => warnLog.push(a.map(String).join(" ")),
  };
  win.BIOROUTER_APP_CONFIG = {
    appId: APP_ID,
    autoChat: false,
    ui: true,
    wsToken: WS_TOKEN,
  };
  // jsdom has no real 2D canvas. Give getContext a no-op stub so the `network`
  // engine's draw path actually runs (proving it never crashes) without needing
  // a real context — physics, selection and positions work regardless.
  win.HTMLCanvasElement.prototype.getContext = function () {
    return {
      save() {},
      restore() {},
      beginPath() {},
      moveTo() {},
      lineTo() {},
      arc() {},
      fill() {},
      stroke() {},
      clearRect() {},
      setTransform() {},
      setLineDash() {},
      fillText() {},
      measureText() {
        return { width: 10 };
      },
      createRadialGradient() {
        return { addColorStop() {} };
      },
    };
  };
  win.eval(bundle);

  const doc = win.document;
  const $ = (id) => doc.getElementById(id);

  check(
    "an unthemed app starts in deterministic light mode",
    doc.documentElement.getAttribute("data-br-theme") === "light",
    doc.documentElement.getAttribute("data-br-theme")
  );

  // Wait until the SDK connected, processed `ready`, and reported its surface
  // (which also indexes the author bindings).
  const up = await waitFor(() => received.some((f) => f.type === "ui_surface"));
  check("SDK connects and reports its surface", up, "no ui_surface frame arrived");
  if (!up) {
    dom.window.close();
    return 1;
  }

  // ── Scenario D (checked early): wsToken rides the WS URL ──────────────────
  check(
    "WS upgrade URL carries token= when config.wsToken is set",
    !!lastUpgradeUrl &&
      lastUpgradeUrl.includes("token=" + WS_TOKEN) &&
      lastUpgradeUrl.includes("client_id="),
    `upgrade url: ${lastUpgradeUrl}`
  );

  // ── Scenario A: state snapshot → patch drives bindings; unsafe sinks refused ─
  emitUi({
    cmd: "state",
    mode: "snapshot",
    version: 3,
    doc: {
      cohort: { count: 7 },
      link: "https://example.org/a",
      danger: "javascript:alert(1)",
      handler: "steal()",
      visible: true,
    },
  });
  await waitFor(() => $("cnt") && $("cnt").textContent === "7");
  check("snapshot: [data-br-bind] span textContent updates", $("cnt").textContent === "7", $("cnt").textContent);
  check(
    "snapshot: [data-br-bind-attr] href updates",
    $("lnk").getAttribute("href") === "https://example.org/a",
    $("lnk").getAttribute("href")
  );
  check(
    "javascript: href is refused (never set)",
    !/javascript:/i.test($("jsl").getAttribute("href") || ""),
    "href=" + $("jsl").getAttribute("href")
  );
  check(
    "unsafe href refusal is logged",
    warnLog.some((w) => /unsafe href/i.test(w)),
    warnLog.join(" | ")
  );
  check(
    "onclick bind-attr is refused (never set)",
    $("clk").getAttribute("onclick") === null,
    "onclick=" + $("clk").getAttribute("onclick")
  );
  check(
    "forbidden on* attr refusal is logged",
    warnLog.some((w) => /forbidden attribute: onclick/i.test(w)),
    warnLog.join(" | ")
  );
  check("bind-show reveals element when truthy", $("box").hidden === false, "hidden=" + $("box").hidden);

  emitUi({
    cmd: "state",
    mode: "patch",
    version: 4,
    patch: [
      { op: "replace", path: "/cohort/count", value: 12 },
      { op: "replace", path: "/link", value: "/rel/path" },
      { op: "replace", path: "/visible", value: false },
    ],
  });
  await waitFor(() => $("cnt").textContent === "12");
  check("patch: bound span updates to new value", $("cnt").textContent === "12", $("cnt").textContent);
  check("patch: bound href updates (relative allowed)", $("lnk").getAttribute("href") === "/rel/path", $("lnk").getAttribute("href"));
  check("patch: bind-show hides element when falsy", $("box").hidden === true, "hidden=" + $("box").hidden);

  // ── Scenario B: br.state.set → state_write(baseVersion); echo patch → subscribe ─
  win.eval(
    "window.__xSub = []; window.__unsub = window.Biorouter.state.subscribe('/x', function (v) { window.__xSub.push(v); });"
  );
  const beforeSet = received.length;
  win.eval("window.Biorouter.state.set('/x', 5);");
  const gotWrite = await waitFor(() =>
    received.slice(beforeSet).some((f) => f.type === "state_write")
  );
  check("state.set emits a state_write frame", gotWrite, "no state_write frame");
  const sw = received.slice(beforeSet).find((f) => f.type === "state_write");
  check(
    "state_write carries set{path,value} and numeric baseVersion",
    !!sw && sw.set && sw.set.path === "/x" && sw.set.value === 5 && typeof sw.baseVersion === "number",
    JSON.stringify(sw)
  );
  check(
    "state_write.baseVersion is the pre-write version (4)",
    !!sw && sw.baseVersion === 4,
    "baseVersion=" + (sw && sw.baseVersion)
  );
  check(
    "optimistic local apply fires the subscriber",
    win.__xSub.length >= 1 && win.__xSub[win.__xSub.length - 1] === 5,
    JSON.stringify(win.__xSub)
  );
  // Server echoes an authoritative patch (correcting /x to 9, bumping version).
  emitUi({ cmd: "state", mode: "patch", version: 5, patch: [{ op: "replace", path: "/x", value: 9 }] });
  await waitFor(() => win.__xSub.some((v) => v === 9));
  check("echoed patch fires subscribe('/x')", win.__xSub.some((v) => v === 9), JSON.stringify(win.__xSub));
  check("state.get('/x') reflects the echoed value", win.eval("window.Biorouter.state.get('/x')") === 9, "get=" + win.eval("window.Biorouter.state.get('/x')"));
  // Unsubscribe stops further notifications.
  const subCountBefore = win.__xSub.length;
  win.eval("window.__unsub();");
  emitUi({ cmd: "state", mode: "patch", version: 6, patch: [{ op: "replace", path: "/x", value: 11 }] });
  await delay(150);
  check("unsubscribe stops notifications", win.__xSub.length === subCountBefore, `before=${subCountBefore} after=${win.__xSub.length}`);

  // ── Scenario C: unknown cmd + unknown widget kind → no crash, placeholder ──
  const countBefore = $("cnt").textContent;
  emitUi({ cmd: "totally_unknown_cmd", foo: 1 });
  await delay(80);
  check("unknown cmd is a silent no-op (client still alive)", !!win.Biorouter && $("cnt").textContent === countBefore, "cnt=" + $("cnt").textContent);

  emitUi({ cmd: "render", target: "@region:results", body: [{ t: "quantum_widget", label: "x" }] });
  await waitFor(() => doc.querySelector("#results-probe, [data-br-region='results'] .br-unknown-widget"));
  const ph = doc.querySelector("[data-br-region='results'] .br-unknown-widget");
  check("unknown widget kind renders a neutral placeholder", !!ph, "no .br-unknown-widget");
  check(
    "placeholder names the unsupported kind",
    !!ph && ph.textContent === "[unsupported: quantum_widget]",
    ph && ph.textContent
  );
  check(
    "unknown widget kind is warned once",
    warnLog.filter((w) => /unsupported widget kind: quantum_widget/i.test(w)).length === 1,
    "warns=" + warnLog.filter((w) => /quantum_widget/i.test(w)).length
  );
  // Rendering the same unknown kind again must NOT warn again (once per kind).
  emitUi({ cmd: "render", target: "@region:results", body: [{ t: "quantum_widget", label: "y" }] });
  await delay(80);
  check(
    "re-rendering the same unknown kind does not re-warn",
    warnLog.filter((w) => /unsupported widget kind: quantum_widget/i.test(w)).length === 1,
    "warns=" + warnLog.filter((w) => /unsupported widget kind: quantum_widget/i.test(w)).length
  );

  // ── Scenario E: legacy `{cmd:"state", state:{…}}` (no mode) → snapshot ─────
  // An old server that predates the mode field must keep working: the whole
  // `state` object replaces the doc and re-paints bindings.
  emitUi({ cmd: "state", state: { cohort: { count: 99 }, link: "https://legacy.example/z", visible: true } });
  await waitFor(() => $("cnt").textContent === "99");
  check("legacy state frame (no mode) is treated as a snapshot", $("cnt").textContent === "99", $("cnt").textContent);
  check("legacy snapshot re-evaluates all bindings", $("lnk").getAttribute("href") === "https://legacy.example/z", $("lnk").getAttribute("href"));

  // ── Scenario F: ui_patch — instance registry, morph, focus preservation ────
  emitUi({
    cmd: "render",
    target: "@region:results",
    body: [
      { t: "kpi", id: "k1", label: "Count", value: 10, delta: "+2", unit: "n" },
      { t: "form", id: "f1", children: [{ t: "input", name: "q", label: "Query" }] },
    ],
  });
  await waitFor(() => doc.querySelector("[data-br-iid='k1']") && doc.querySelector("[data-br-iid='f1']"));
  check(
    "ui_render tags data-br-iid and registers instances",
    !!doc.querySelector("[data-br-iid='k1']") && !!doc.querySelector("[data-br-iid='f1']")
  );
  {
    const d = doc.querySelector("[data-br-iid='k1'] .br-kpi__delta");
    check("kpi renders a delta with an up/down arrow", !!d && /▲|▼/.test(d.textContent || ""), d && d.textContent);
  }

  // Focus a sibling input, patch a different instance; the focus must survive.
  const qInput = doc.querySelector("[data-br-iid='f1'] input");
  if (qInput && typeof qInput.focus === "function") qInput.focus();
  check("sibling input can take focus", doc.activeElement === qInput);
  emitUi({ cmd: "patch", ops: [{ op: "set_props", id: "k1", props: { value: 99, delta: "-5" } }] });
  await waitFor(() => {
    const v = doc.querySelector("[data-br-iid='k1'] .br-kpi__value");
    return v && (v.textContent || "").indexOf("99") >= 0;
  });
  check(
    "set_props shallow-merges + re-renders the targeted instance",
    (doc.querySelector("[data-br-iid='k1'] .br-kpi__value").textContent || "").indexOf("99") >= 0
  );
  check("set_props on a sibling preserves focus", doc.activeElement === qInput);
  check("set_props delta now reads as a decrease (▼)", /▼/.test(doc.querySelector("[data-br-iid='k1'] .br-kpi__delta").textContent || ""));

  // replace re-renders the instance in place (same id).
  emitUi({ cmd: "patch", ops: [{ op: "replace", id: "k1", node: { t: "kpi", label: "Total", value: 7 } }] });
  await waitFor(() => {
    const l = doc.querySelector("[data-br-iid='k1'] .br-kpi__label");
    return l && l.textContent === "Total";
  });
  check("replace re-renders an instance in place", doc.querySelector("[data-br-iid='k1'] .br-kpi__label").textContent === "Total");

  // add + remove round-trip.
  emitUi({ cmd: "patch", ops: [{ op: "add", id: "added1", target: "@region:results", node: { t: "text", value: "hello-added" } }] });
  await waitFor(() => doc.querySelector("[data-br-iid='added1']"));
  check("add inserts a new instance at the target", (doc.querySelector("[data-br-iid='added1']").textContent || "").indexOf("hello-added") >= 0);
  emitUi({ cmd: "patch", ops: [{ op: "remove", id: "added1" }] });
  await waitFor(() => !doc.querySelector("[data-br-iid='added1']"));
  check("remove deletes the instance + registry entry", !doc.querySelector("[data-br-iid='added1']"));

  // ── Scenario G: log append + cap (drop oldest) ─────────────────────────────
  emitUi({
    cmd: "render",
    target: "@region:results",
    body: [{ t: "log", id: "log1", max: 3, lines: [{ text: "l1" }, { text: "l2" }] }],
  });
  await waitFor(() => doc.querySelectorAll("[data-br-iid='log1'] .br-log__line").length === 2);
  check("log renders its initial lines", doc.querySelectorAll("[data-br-iid='log1'] .br-log__line").length === 2);
  emitUi({
    cmd: "patch",
    ops: [{ op: "set_props", id: "log1", props: { append: [{ text: "l3" }, { text: "l4" }, { text: "l5" }] } }],
  });
  await waitFor(() => doc.querySelectorAll("[data-br-iid='log1'] .br-log__line").length === 3);
  {
    const rows = Array.prototype.map.call(doc.querySelectorAll("[data-br-iid='log1'] .br-log__line"), (e) => e.textContent);
    check("log append caps at node.max, dropping the oldest", rows.length === 3 && rows[2] === "l5" && rows.indexOf("l1") < 0, rows.join(","));
  }

  // ── Scenario H: plot scatter + heatmap render SVG ──────────────────────────
  emitUi({
    cmd: "render",
    target: "@region:results",
    body: [
      { t: "plot", id: "sc1", spec: { type: "scatter", data: [{ x: 1, y: 2 }, { x: 3, y: 4 }, { x: 5, y: 1 }] } },
      { t: "plot", id: "hm1", spec: { type: "heatmap", z: [[0, 1], [2, 3]] } },
    ],
  });
  await waitFor(() => doc.querySelector("[data-br-iid='sc1'] svg") && doc.querySelector("[data-br-iid='hm1'] svg"));
  check("plot scatter produces an svg with one circle per point", doc.querySelectorAll("[data-br-iid='sc1'] circle").length === 3);
  check("plot heatmap produces an svg grid of rects", doc.querySelectorAll("[data-br-iid='hm1'] rect").length >= 4);

  // ── Scenario I: network engine mounts, selects, exposes positions ──────────
  emitUi({
    cmd: "render",
    target: "@region:results",
    body: [
      {
        t: "network",
        id: "net1",
        spec: {
          nodes: [{ id: "n1", label: "A" }, { id: "n2", label: "B" }, { id: "n3", label: "C" }],
          edges: [{ source: "n1", target: "n2" }, { source: "n2", target: "n3" }],
        },
      },
    ],
  });
  await waitFor(() => doc.querySelector("[data-br-iid='net1'] canvas"));
  check("network mounts a canvas", !!doc.querySelector("[data-br-iid='net1'] canvas"));
  const netCanvas = doc.querySelector("[data-br-iid='net1'] canvas");
  let netSel = "none";
  netCanvas.addEventListener("br-network-select", (e) => {
    netSel = e && e.detail ? e.detail.id : null;
  });
  win.Biorouter.ui.network("net1").select("n1");
  check("programmatic select fires a br-network-select CustomEvent", netSel === "n1", "detail.id=" + netSel);
  const netPos = win.Biorouter.ui.network("net1").positions();
  check(
    "network exposes a positions object keyed by node id",
    !!netPos && typeof netPos === "object" && !!netPos.n1 && typeof netPos.n1.x === "number",
    JSON.stringify(netPos && netPos.n1)
  );

  // ── Scenario J: author component registry — mount + update ─────────────────
  win.eval(
    "window.__cMount = []; window.__cUpdate = [];" +
      "window.Biorouter.components.register('greeter', {" +
      "  mount: function (el, props) { window.__cMount.push(props && props.name); el.setAttribute('data-greet', (props && props.name) || ''); el.textContent = 'hi ' + ((props && props.name) || ''); }," +
      "  update: function (el, props) { window.__cUpdate.push(props && props.name); el.setAttribute('data-greet', (props && props.name) || ''); el.textContent = 'hi ' + ((props && props.name) || ''); }" +
      "});"
  );
  emitUi({ cmd: "render", target: "@region:results", body: [{ t: "component", id: "c1", name: "greeter", props: { name: "Ada" } }] });
  await waitFor(() => doc.querySelector("[data-br-iid='c1'][data-greet='Ada']"));
  check("component mount runs with the agent-supplied props", win.__cMount.length === 1 && win.__cMount[0] === "Ada", JSON.stringify(win.__cMount));
  emitUi({ cmd: "patch", ops: [{ op: "set_props", id: "c1", props: { name: "Grace" } }] });
  await waitFor(() => doc.querySelector("[data-br-iid='c1'][data-greet='Grace']"));
  check("component update runs with new props (container preserved)", win.__cUpdate.length === 1 && win.__cUpdate[0] === "Grace", JSON.stringify(win.__cUpdate));

  // ── Scenario K: privileged html + figure nodes ─────────────────────────────
  emitUi({
    cmd: "render",
    target: "@region:results",
    body: [
      { t: "html", id: "h1", html: "<b class='br-sanit'>safe-html</b>" },
      { t: "figure", id: "fig1", html: "<!doctype html><body>fig</body>" },
    ],
  });
  await waitFor(() => doc.querySelector("[data-br-iid='h1'] .br-sanit") && doc.querySelector("[data-br-iid='fig1'] iframe"));
  check("html node lands its (server-sanitized) innerHTML", !!doc.querySelector("[data-br-iid='h1'] .br-sanit"));
  {
    // Framed from the daemon's figure shell, never `srcdoc`: a srcdoc document
    // inherits the app page's CSP, which refuses the figure's inline scripts.
    // (jsdom runs no frame, so the ready/figure handshake is checked in a real
    // browser, not here.)
    const fr = doc.querySelector("[data-br-iid='fig1'] iframe");
    check(
      "figure node frames the app's figure shell, sandboxed, without srcdoc",
      !!fr &&
        fr.getAttribute("sandbox") === "allow-scripts" &&
        fr.getAttribute("src") === "/apps/" + encodeURIComponent(APP_ID) + "/figure-frame" &&
        !fr.hasAttribute("srcdoc"),
      fr && fr.outerHTML.slice(0, 200)
    );
    // A figure reports its height as `{ payload: { height } }` (`reportSize` in
    // the autovis `_common.js`), and the frame grows to it. The SDK read a bare
    // `height`, which no figure sends, so every figure stayed clipped at the
    // frame's minimum (measured in Chrome).
    const fwin = fr && fr.contentWindow;
    if (fwin) {
      win.dispatchEvent(
        new win.MessageEvent("message", {
          data: { type: "ui-size-change", payload: { height: 640 } },
          source: fwin,
        })
      );
    }
    check(
      "a figure's reported height grows its frame",
      !!fwin && fr.style.height === "640px",
      fr && `contentWindow=${!!fwin} height=${fr.style.height}`
    );
    // The shell is sent the figure each time it says it is ready. A frame
    // moved in the DOM (`ui_layout` moving a panel) reloads, and the SDK used to
    // answer only the first ready, so the reloaded shell stayed empty.
    const posted = [];
    if (fwin) {
      fwin.postMessage = (message, target) => posted.push({ message, target });
      for (let i = 0; i < 2; i++) {
        win.dispatchEvent(
          new win.MessageEvent("message", { data: { type: "biorouter-figure-ready" }, source: fwin })
        );
      }
      // A ready from any other window is not the frame's, and is not answered.
      win.dispatchEvent(new win.MessageEvent("message", { data: { type: "biorouter-figure-ready" }, source: win }));
    }
    check(
      "a figure frame that reloads is sent its figure again",
      posted.length === 2 &&
        posted.every(
          (p) =>
            p.target === "*" &&
            p.message &&
            p.message.type === "biorouter-figure" &&
            p.message.html === "<!doctype html><body>fig</body>"
        ),
      JSON.stringify(posted)
    );
  }

  // ── Scenario L: app.actions — app_call round-trip, errors, truncation ──────
  win.eval(
    "window.Biorouter.actions.register('greet', function (args) { return { hello: (args && args.who) || '?' }; });" +
      "window.Biorouter.actions.register('boom', function () { throw new Error('kaboom'); });" +
      "window.Biorouter.actions.register('big', function () { return { blob: new Array(70001).join('x') }; });" +
      "window.Biorouter.actions.register('slow', function (args) { return Promise.resolve({ doubled: (args && args.n) * 2 }); });"
  );
  emitUi({ cmd: "app_call", callId: "ac1", action: "greet", args: { who: "Ada" } });
  await waitFor(() => received.some((f) => f.type === "app_result" && f.callId === "ac1"));
  {
    const r = received.find((f) => f.type === "app_result" && f.callId === "ac1");
    check("app_call dispatches to the registered handler and returns its result", !!r && r.result && r.result.hello === "Ada", JSON.stringify(r));
  }
  // An async handler resolves before app_result is sent.
  emitUi({ cmd: "app_call", callId: "ac5", action: "slow", args: { n: 21 } });
  await waitFor(() => received.some((f) => f.type === "app_result" && f.callId === "ac5"));
  {
    const r = received.find((f) => f.type === "app_result" && f.callId === "ac5");
    check("an async (Promise) handler's resolved value is awaited", !!r && r.result && r.result.doubled === 42, JSON.stringify(r));
  }
  emitUi({ cmd: "app_call", callId: "ac2", action: "nope" });
  await waitFor(() => received.some((f) => f.type === "app_result" && f.callId === "ac2"));
  {
    const r = received.find((f) => f.type === "app_result" && f.callId === "ac2");
    check("an unregistered action yields an app_result error", !!r && r.error === "no handler registered for nope", JSON.stringify(r));
  }
  emitUi({ cmd: "app_call", callId: "ac3", action: "boom" });
  await waitFor(() => received.some((f) => f.type === "app_result" && f.callId === "ac3"));
  {
    const r = received.find((f) => f.type === "app_result" && f.callId === "ac3");
    check("a throwing handler yields an app_result error string", !!r && r.error === "kaboom", JSON.stringify(r));
  }
  // The same throw ALSO rides the ui_error feedback loop (where:"action:<name>").
  const gotActionErr = await waitFor(() => received.some((f) => f.type === "ui_error" && f.where === "action:boom"));
  {
    const ue = received.find((f) => f.type === "ui_error" && f.where === "action:boom");
    check("a throwing action ALSO posts a ui_error {where:action:boom, message}", gotActionErr && !!ue && typeof ue.message === "string" && ue.message.length > 0, JSON.stringify(ue));
  }
  emitUi({ cmd: "app_call", callId: "ac4", action: "big" });
  await waitFor(() => received.some((f) => f.type === "app_result" && f.callId === "ac4"));
  {
    const r = received.find((f) => f.type === "app_result" && f.callId === "ac4");
    const wrapped = r && r.result;
    check(
      "an oversized result is truncated inside a {truncated,text} wrapper",
      !!wrapped && wrapped.truncated === true && typeof wrapped.text === "string" && wrapped.text.length <= 64 * 1024 + 32 && /…\[truncated\]$/.test(wrapped.text),
      JSON.stringify({ truncated: wrapped && wrapped.truncated, len: wrapped && (wrapped.text || "").length })
    );
  }
  check(
    "br.actions.list() reports the registered action names",
    win.eval("window.Biorouter.actions.list().indexOf('greet') >= 0 && window.Biorouter.actions.list().indexOf('boom') >= 0"),
    win.eval("JSON.stringify(window.Biorouter.actions.list())")
  );

  // ── Scenario M: br.call — typed turns with structured / prose results ──────
  win.eval(
    "window.__callResult = null;" +
      "window.Biorouter.call({ name: 'analyze', args: { x: 1 }, outputSchema: { type: 'object' } }).then(function (r) { window.__callResult = r; });"
  );
  await waitFor(() => received.some((f) => f.type === "call"));
  const callFrame = received.find((f) => f.type === "call");
  check("br.call sends a call frame with a fresh callId", !!callFrame && typeof callFrame.callId === "string" && callFrame.callId.length > 0, JSON.stringify(callFrame));
  check(
    "call frame carries name/args/outputSchema (name+args form)",
    !!callFrame && callFrame.name === "analyze" && callFrame.args && callFrame.args.x === 1 && !!callFrame.outputSchema,
    JSON.stringify(callFrame)
  );
  send({ type: "output", callId: callFrame.callId, value: { score: 42 } });
  await waitFor(() => win.eval("window.__callResult") != null);
  check(
    "br.call resolves {value} when a matching output frame arrives",
    win.eval("window.__callResult && window.__callResult.value && window.__callResult.value.score") === 42,
    win.eval("JSON.stringify(window.__callResult)")
  );

  win.eval(
    "window.__callText = null;" +
      "window.Biorouter.call({ text: 'summarize the cohort' }).then(function (r) { window.__callText = r; });"
  );
  await waitFor(() => received.filter((f) => f.type === "call").length >= 2);
  const callFrame2 = received.filter((f) => f.type === "call").pop();
  check("text-only br.call sends a call frame with text and no name", !!callFrame2 && callFrame2.text === "summarize the cohort" && callFrame2.name === undefined, JSON.stringify(callFrame2));
  send({ type: "message", delta: "Cohort " });
  send({ type: "message", delta: "has 7 rows." });
  send({ type: "done" });
  await waitFor(() => win.eval("window.__callText") != null);
  check(
    "br.call resolves {text} when the turn ends (done) without an output frame",
    win.eval("window.__callText && window.__callText.text") === "Cohort has 7 rows.",
    win.eval("JSON.stringify(window.__callText)")
  );

  // Supersede: a second superseding call on the same key cancels the first.
  const baseSup = received.length;
  win.eval(
    "window.__sup1 = null; window.__sup2 = null;" +
      "window.Biorouter.call({ name: 'search', args: { q: 'a' }, supersede: true }).then(function (r) { window.__sup1 = r; });" +
      "window.Biorouter.call({ name: 'search', args: { q: 'ab' }, supersede: true }).then(function (r) { window.__sup2 = r; });"
  );
  await waitFor(() => win.eval("window.__sup1") != null);
  check("a superseded br.call resolves {superseded:true}", win.eval("window.__sup1 && window.__sup1.superseded") === true, win.eval("JSON.stringify(window.__sup1)"));
  const supFrames = received.slice(baseSup);
  check("supersede sends a cancel frame for the in-flight turn", supFrames.some((f) => f.type === "cancel"), JSON.stringify(supFrames.map((f) => f.type)));
  check("supersede still sends both call frames (old + new)", supFrames.filter((f) => f.type === "call" && f.name === "search").length === 2, JSON.stringify(supFrames.map((f) => f.type)));

  // ── Scenario N: app.signals — declared coalescing, trailing-edge, last-wins ─
  send({
    type: "ready",
    protocol: 2,
    capabilities: ["ui"],
    sessionId: "harness",
    resumed: false,
    surface: {
      signals: [
        { name: "filter_changed", coalesceMs: 40 },
        { name: "node_selected", coalesceMs: 30 },
      ],
      actions: ["greet"],
    },
  });
  await waitFor(() => win.eval("window.Biorouter.signals.declared().length") >= 2);
  check(
    "the ready surface's signals are latched into signals.declared()",
    win.eval("window.Biorouter.signals.declared().map(function (s) { return s.name; }).indexOf('filter_changed') >= 0"),
    win.eval("JSON.stringify(window.Biorouter.signals.declared())")
  );
  const baseSig = received.length;
  win.eval(
    "window.Biorouter.signals.emit('filter_changed', { q: 'first' });" +
      "window.Biorouter.signals.emit('filter_changed', { q: 'second' });"
  );
  await waitFor(() => received.slice(baseSig).some((f) => f.type === "signal" && f.name === "filter_changed"), 2000);
  await delay(80);
  {
    const sigs = received.slice(baseSig).filter((f) => f.type === "signal" && f.name === "filter_changed");
    check("two rapid emits within the declared window coalesce to ONE signal frame", sigs.length === 1, "count=" + sigs.length);
    check("the coalesced signal carries the LAST payload", sigs[0] && sigs[0].payload && sigs[0].payload.q === "second", JSON.stringify(sigs[0]));
  }
  // An UNdeclared signal falls back to the 250ms default (proves declared wins).
  const baseUn = received.length;
  win.eval(
    "window.Biorouter.signals.emit('mystery', { v: 1 });" +
      "window.Biorouter.signals.emit('mystery', { v: 2 });"
  );
  await delay(80);
  check(
    "an undeclared signal uses the 250ms default (no frame yet at 80ms)",
    !received.slice(baseUn).some((f) => f.type === "signal" && f.name === "mystery"),
    "names=" + JSON.stringify(received.slice(baseUn).filter((f) => f.type === "signal").map((f) => f.name))
  );
  await waitFor(() => received.slice(baseUn).some((f) => f.type === "signal" && f.name === "mystery"), 1500);
  {
    const myst = received.slice(baseUn).filter((f) => f.type === "signal" && f.name === "mystery");
    check("the undeclared signal eventually sends ONE frame with the last payload", myst.length === 1 && myst[0].payload && myst[0].payload.v === 2, JSON.stringify(myst));
  }

  // ── Scenario O: network selection auto-emits the declared node_selected ─────
  const baseNode = received.length;
  emitUi({
    cmd: "render",
    target: "@region:results",
    body: [
      {
        t: "network",
        id: "net2",
        spec: { nodes: [{ id: "x1", label: "X" }, { id: "x2", label: "Y" }], edges: [{ source: "x1", target: "x2" }] },
      },
    ],
  });
  await waitFor(() => doc.querySelector("[data-br-iid='net2'] canvas"));
  win.eval("window.Biorouter.ui.network('net2').select('x1');");
  await waitFor(() => received.slice(baseNode).some((f) => f.type === "signal" && f.name === "node_selected"), 2000);
  {
    const nodeSig = received.slice(baseNode).filter((f) => f.type === "signal" && f.name === "node_selected");
    check("selecting a network node auto-emits a node_selected signal (declared)", nodeSig.length >= 1, JSON.stringify(nodeSig.map((f) => f.payload)));
    check(
      "the node_selected payload carries {id, instance}",
      nodeSig[0] && nodeSig[0].payload && nodeSig[0].payload.id === "x1" && nodeSig[0].payload.instance === "net2",
      JSON.stringify(nodeSig[0] && nodeSig[0].payload)
    );
  }

  // ── Scenario P: br.kb — grant gating, search/page/ingest, timeout ──────────
  // At this point the last `ready` advertised only ["ui"] (scenario N), so the
  // KB grant is absent: a kb call must reject immediately without a round-trip.
  const baseNoGrant = received.length;
  win.eval(
    "window.__kbNoGrant = 'pending';" +
      "window.Biorouter.kb.search('x').then(" +
      "  function () { window.__kbNoGrant = 'resolved'; }," +
      "  function (e) { window.__kbNoGrant = String(e && e.message); });"
  );
  await waitFor(() => win.eval("window.__kbNoGrant") !== "pending");
  check(
    "br.kb rejects immediately with a clear error when no KB grant is advertised",
    /knowledge-base grant/i.test(win.eval("window.__kbNoGrant")),
    win.eval("window.__kbNoGrant")
  );
  check(
    "an ungranted br.kb call sends NO kb frame (pure pre-reject)",
    !received.slice(baseNoGrant).some((f) => f.type === "kb"),
    JSON.stringify(received.slice(baseNoGrant).map((f) => f.type))
  );

  // Advertise the `data` grant (what manifest.advertised() emits for a KB
  // source); the KB methods now round-trip to the server.
  send({ type: "ready", protocol: 2, capabilities: ["ui", "data"], sessionId: "harness", resumed: false });
  await waitFor(() => win.eval("window.Biorouter.has('data')"));
  check("a ready frame advertising 'data' lifts the KB grant gate", win.eval("window.Biorouter.has('data')") === true);

  // Scenario P1: search resolves.
  const baseSearch = received.length;
  win.eval(
    "window.__search = null;" +
      "window.Biorouter.kb.search('cohort', { limit: 5 }).then(" +
      "  function (r) { window.__search = r; }," +
      "  function (e) { window.__search = { err: String(e.message) }; });"
  );
  await waitFor(() => received.slice(baseSearch).some((f) => f.type === "kb" && f.op === "search"));
  const sf = received.slice(baseSearch).find((f) => f.type === "kb" && f.op === "search");
  check(
    "br.kb.search sends a kb frame {op:search, params.query/limit, fresh reqId}",
    !!sf && sf.op === "search" && sf.params && sf.params.query === "cohort" && sf.params.limit === 5 && typeof sf.reqId === "string" && sf.reqId.length > 0,
    JSON.stringify(sf)
  );
  send({ type: "kb_result", reqId: sf.reqId, result: { hits: [{ path: "a.md", score: 0.9 }] } });
  await waitFor(() => win.eval("window.__search") != null);
  check(
    "br.kb.search resolves with the server's result on kb_result",
    win.eval("window.__search && window.__search.hits && window.__search.hits.length") === 1,
    win.eval("JSON.stringify(window.__search)")
  );

  // Scenario P2: error rejects with Error(error).
  const baseErr = received.length;
  win.eval(
    "window.__pageErr = null;" +
      "window.Biorouter.kb.page('missing.md').then(" +
      "  function (r) { window.__pageErr = { ok: r }; }," +
      "  function (e) { window.__pageErr = String(e.message); });"
  );
  await waitFor(() => received.slice(baseErr).some((f) => f.type === "kb" && f.op === "page"));
  const pf = received.slice(baseErr).find((f) => f.type === "kb" && f.op === "page");
  check("br.kb.page sends {op:page, params.path}", !!pf && pf.params && pf.params.path === "missing.md", JSON.stringify(pf));
  send({ type: "kb_result", reqId: pf.reqId, error: "page not found" });
  await waitFor(() => win.eval("window.__pageErr") != null);
  check(
    "a kb_result carrying an error rejects with Error(error)",
    win.eval("window.__pageErr") === "page not found",
    win.eval("JSON.stringify(window.__pageErr)")
  );

  // Scenario P3: ingest streams two progress events, then the final result.
  const baseIng = received.length;
  win.eval(
    "window.__ingProg = []; window.__ingDone = null;" +
      "window.Biorouter.kb.ingest([{ url: 'https://x' }], {" +
      "  onProgress: function (p) { window.__ingProg.push(p.stage + ':' + (p.pct == null ? '-' : p.pct)); } })" +
      ".then(function (r) { window.__ingDone = r; }, function (e) { window.__ingDone = { err: String(e.message) }; });"
  );
  await waitFor(() => received.slice(baseIng).some((f) => f.type === "kb" && f.op === "ingest"));
  const inf = received.slice(baseIng).find((f) => f.type === "kb" && f.op === "ingest");
  check(
    "br.kb.ingest sends {op:ingest, params.items}",
    !!inf && inf.params && Array.isArray(inf.params.items) && inf.params.items.length === 1,
    JSON.stringify(inf)
  );
  send({ type: "kb_progress", reqId: inf.reqId, stage: "fetch", pct: 10 });
  send({ type: "kb_progress", reqId: inf.reqId, stage: "digest", detail: "page 1", pct: 60 });
  await waitFor(() => win.eval("window.__ingProg.length") >= 2);
  check(
    "br.kb.ingest routes each kb_progress frame to onProgress, in order",
    win.eval("window.__ingProg.join('|')") === "fetch:10|digest:60",
    win.eval("window.__ingProg.join('|')")
  );
  check("br.kb.ingest does not resolve before its final kb_result", win.eval("window.__ingDone") == null);
  send({ type: "kb_result", reqId: inf.reqId, result: { added: 1 } });
  await waitFor(() => win.eval("window.__ingDone") != null);
  check(
    "br.kb.ingest resolves with the final kb_result after the progress stream",
    win.eval("window.__ingDone && window.__ingDone.added") === 1,
    win.eval("JSON.stringify(window.__ingDone)")
  );

  // Scenario P4: timeout rejects (100ms per-call override; no reply is sent).
  const baseTo = received.length;
  win.eval(
    "window.__toRes = null;" +
      "window.Biorouter.kb.search('slow', { timeoutMs: 100 }).then(" +
      "  function (r) { window.__toRes = { ok: r }; }," +
      "  function (e) { window.__toRes = String(e.message); });"
  );
  await waitFor(() => received.slice(baseTo).some((f) => f.type === "kb" && f.op === "search"));
  await waitFor(() => win.eval("window.__toRes") != null, 1500);
  check(
    "br.kb rejects with a timeout Error when no reply arrives within timeoutMs",
    typeof win.eval("window.__toRes") === "string" && /timed out/i.test(win.eval("window.__toRes")),
    win.eval("JSON.stringify(window.__toRes)")
  );

  // ── Scenario Q: br.model.status() round-trip ───────────────────────────────
  const baseMs = received.length;
  win.eval(
    "window.__ms = null;" +
      "window.Biorouter.model.status().then(" +
      "  function (s) { window.__ms = s; }, function (e) { window.__ms = { err: String(e.message) }; });"
  );
  await waitFor(() => received.slice(baseMs).some((f) => f.type === "model_status"));
  check(
    "br.model.status() sends a model_status request frame (no params)",
    received.slice(baseMs).some((f) => f.type === "model_status"),
    JSON.stringify(received.slice(baseMs).map((f) => f.type))
  );
  send({ type: "model_status", provider: "llamacpp", model: "qwen3.5-4b", ready: false, detail: "downloading 42%" });
  await waitFor(() => win.eval("window.__ms") != null);
  check(
    "br.model.status() resolves {provider,model,ready,detail} from the reply",
    win.eval("window.__ms && window.__ms.provider") === "llamacpp" &&
      win.eval("window.__ms.model") === "qwen3.5-4b" &&
      win.eval("window.__ms.ready") === false &&
      /downloading/.test(win.eval("String(window.__ms && window.__ms.detail)")),
    win.eval("JSON.stringify(window.__ms)")
  );

  // ── Scenario R: br.kb.graphToNetwork — pure KB-graph → network-spec mapping ─
  win.eval(
    "window.__net = window.Biorouter.kb.graphToNetwork({" +
      "  nodes: [" +
      "    { id: 'n1', label: 'Gene A', kind: 'Gene' }," +
      "    { id: 'n2', label: 'Disease B', kind: 'Disease' }," +
      "    { id: 'n3', label: 'Untyped' }," + // missing type → tolerated
      "    'n4'," + // bare string node → id === label
      "    { label: 'no-id' }" + // dropped: no usable id
      "  ]," +
      "  edges: [" +
      "    { from: 'n1', to: 'n2', relation: 'associated_with' }," +
      "    { source: 'n2', target: 'n3' }," + // alt field names, no kind
      "    { from: 'n1' }" + // dropped: no target
      "  ]" +
      "});"
  );
  const netDump = win.eval("JSON.stringify(window.__net)");
  check("graphToNetwork drops id-less nodes (4 of 5 map)", win.eval("window.__net.nodes.length") === 4, netDump);
  check(
    "graphToNetwork maps node kind→type",
    win.eval("window.__net.nodes[0].type") === "Gene" && win.eval("window.__net.nodes[1].type") === "Disease",
    netDump
  );
  check(
    "graphToNetwork tolerates a missing-type node (type undefined, label kept)",
    win.eval("typeof window.__net.nodes[2].type") === "undefined" && win.eval("window.__net.nodes[2].label") === "Untyped",
    netDump
  );
  check(
    "graphToNetwork accepts a bare string node (id === label)",
    win.eval("window.__net.nodes[3].id") === "n4" && win.eval("window.__net.nodes[3].label") === "n4",
    netDump
  );
  check(
    "graphToNetwork maps edges to {source,target,kind} across from/to & source/target (drops incomplete)",
    win.eval("window.__net.edges.length") === 2 &&
      win.eval("window.__net.edges[0].source") === "n1" &&
      win.eval("window.__net.edges[0].target") === "n2" &&
      win.eval("window.__net.edges[0].kind") === "associated_with",
    netDump
  );
  check(
    "graphToNetwork tolerates an edge with no kind (kind undefined)",
    win.eval("window.__net.edges[1].source") === "n2" &&
      win.eval("window.__net.edges[1].target") === "n3" &&
      win.eval("typeof window.__net.edges[1].kind") === "undefined",
    netDump
  );
  check("graphToNetwork is defensive about a non-graph input (empty spec)", win.eval("(function(){var n=window.Biorouter.kb.graphToNetwork(null);return n.nodes.length===0&&n.edges.length===0;})()"));

  // ── Scenario S: theme pack frame sets data-br-pack; unknown ignored ────────
  emitUi({ cmd: "theme", pack: "clinical" });
  await waitFor(() => doc.documentElement.getAttribute("data-br-pack") === "clinical");
  check(
    "theme pack sets data-br-pack on <html>",
    doc.documentElement.getAttribute("data-br-pack") === "clinical",
    doc.documentElement.getAttribute("data-br-pack")
  );
  check(
    "theme pack clears the generic mode so its palette can win",
    doc.documentElement.getAttribute("data-br-theme") === null,
    doc.documentElement.getAttribute("data-br-theme")
  );
  {
    const warnBefore = warnLog.length;
    emitUi({ cmd: "theme", pack: "nonsense-pack" });
    await delay(80);
    check(
      "an unknown theme pack is ignored (data-br-pack unchanged)",
      doc.documentElement.getAttribute("data-br-pack") === "clinical",
      doc.documentElement.getAttribute("data-br-pack")
    );
    check(
      "an unknown theme pack is warned",
      warnLog.slice(warnBefore).some((w) => /unknown theme pack/i.test(w)),
      warnLog.slice(warnBefore).join(" | ")
    );
  }

  // ── Scenario T: layout areas → grid; missing named area warns once ─────────
  {
    const warnBefore = warnLog.length;
    emitUi({ cmd: "layout", areas: ["header header", "sidebar results"], sizes: ["200px", "1fr"] });
    await waitFor(() => (doc.body.style.getPropertyValue("grid-template-areas") || "").indexOf("results") >= 0);
    const areas = doc.body.style.getPropertyValue("grid-template-areas") || "";
    check(
      "layout areas builds grid-template-areas on the main container",
      /"header header"/.test(areas) && /"sidebar results"/.test(areas),
      areas
    );
    check("layout sets display:grid on the main container", doc.body.style.getPropertyValue("display") === "grid", doc.body.style.getPropertyValue("display"));
    const cols = doc.body.style.getPropertyValue("grid-template-columns") || "";
    check("layout sizes → grid-template-columns (defaults honored)", cols.indexOf("200px") >= 0 && cols.indexOf("1fr") >= 0, cols);
    check(
      "a named area with a matching [data-br-region] gets grid-area assigned",
      (doc.querySelector('[data-br-region="results"]').style.getPropertyValue("grid-area") || "") === "results",
      doc.querySelector('[data-br-region="results"]').style.getPropertyValue("grid-area")
    );
    check(
      "a missing named area (header/sidebar) warns",
      warnLog.slice(warnBefore).some((w) => /layout area has no matching element/i.test(w)),
      warnLog.slice(warnBefore).join(" | ")
    );
  }

  // ── Scenario U: presence chip appears on a panel frame; narrate is verbatim ─
  emitUi({ cmd: "panel", id: "pres1", title: "Cohort", place: "@region:results", body: [{ t: "text", value: "hi" }] });
  await waitFor(() => doc.querySelector("[data-br-panel='pres1']"));
  {
    const chip = doc.querySelector(".br-presence");
    check("a panel frame flashes the presence chip", !!chip && chip.classList.contains("br-presence--on"), chip && chip.className);
    const text = win.eval("window.Biorouter.ui.presenceText()");
    check("derived presence text is 'AI · updating panel <title>'", text === "AI · updating panel Cohort", JSON.stringify(text));
  }
  emitUi({ cmd: "render", target: "@region:results", body: [{ t: "text", value: "x" }], narrate: "Building your cohort view" });
  await waitFor(() => win.eval("window.Biorouter.ui.presenceText()") === "Building your cohort view");
  {
    const chip = doc.querySelector(".br-presence");
    check("a frame's narrate string is shown verbatim in the presence chip", (chip && chip.textContent) === "Building your cohort view" && win.eval("window.Biorouter.ui.presenceText()") === "Building your cohort view", chip && chip.textContent);
  }
  check(
    "br.ui.presence(msg) lets authors show the chip directly",
    (function () {
      win.eval("window.Biorouter.ui.presence('Author note here');");
      return doc.querySelector(".br-presence").textContent === "Author note here" && win.eval("window.Biorouter.ui.presenceText()") === "Author note here";
    })(),
    doc.querySelector(".br-presence") && doc.querySelector(".br-presence").textContent
  );

  // ── Scenario V: ui_suggest — chips render, click sends prompt, x/Escape drop ─
  emitUi({
    cmd: "suggest",
    target: "@region:results",
    chips: [
      { label: "Show KM", prompt: "show the kaplan meier" },
      { label: "Filter by age" },
      { label: "c3", prompt: "c3" },
      { label: "c4", prompt: "c4" },
      { label: "c5", prompt: "c5" },
      { label: "c6-over-cap", prompt: "c6" },
      { label: "c7-over-cap", prompt: "c7" },
    ],
  });
  await waitFor(() => doc.querySelector(".br-suggest"));
  {
    const chips = doc.querySelectorAll(".br-suggest .br-suggest__chip");
    check("ui_suggest renders chip buttons capped at 5 (7 requested → 5)", chips.length === 5, "count=" + chips.length);
    check("ui_suggest renders a dismiss (×) control", !!doc.querySelector(".br-suggest .br-suggest__x"));
  }
  const baseSuggest = received.length;
  win.eval("document.querySelector('.br-suggest .br-suggest__chip').click();");
  await waitFor(() => received.slice(baseSuggest).some((f) => f.type === "prompt"));
  {
    const pf = received.slice(baseSuggest).find((f) => f.type === "prompt");
    check("clicking a chip sends its prompt via br.prompt", !!pf && pf.text === "show the kaplan meier", JSON.stringify(pf));
    check("clicking a chip dismisses the suggestion row", !doc.querySelector(".br-suggest"), "row still present");
  }
  // Escape dismisses a fresh row.
  emitUi({ cmd: "suggest", target: "@region:results", chips: [{ label: "again", prompt: "again" }] });
  await waitFor(() => doc.querySelector(".br-suggest"));
  win.eval("document.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape' }));");
  await waitFor(() => !doc.querySelector(".br-suggest"));
  check("Escape dismisses the suggestion row", !doc.querySelector(".br-suggest"));
  // The × button dismisses too.
  emitUi({ cmd: "suggest", target: "@region:results", chips: [{ label: "z", prompt: "z" }] });
  await waitFor(() => doc.querySelector(".br-suggest"));
  win.eval("document.querySelector('.br-suggest .br-suggest__x').click();");
  await waitFor(() => !doc.querySelector(".br-suggest"));
  check("the × control dismisses the suggestion row", !doc.querySelector(".br-suggest"));

  // ── Scenario W: ui_error feedback loop — placeholder + one frame, rate limit ─
  // Jump the SDK's clock past the 30s ui_error window so this block starts with a
  // fresh rate-limit budget (an earlier action throw already used one slot).
  const advance = (ms) => {
    const base = win.Date.now();
    win.Date.now = () => base + ms;
  };
  advance(60000);
  // W1: a single throwing component mount → neutral placeholder + exactly one ui_error.
  win.eval("window.Biorouter.components.register('crashOne', { mount: function () { throw new Error('mount-boom'); } });");
  const baseW1 = received.length;
  emitUi({ cmd: "render", target: "@region:results", body: [{ t: "component", id: "co1", name: "crashOne", props: {} }] });
  await waitFor(() => doc.querySelector(".br-unknown-widget[data-br-iid='co1']"));
  check("a throwing component mount renders the neutral placeholder in place", !!doc.querySelector(".br-unknown-widget[data-br-iid='co1']"));
  {
    const errs = received.slice(baseW1).filter((f) => f.type === "ui_error" && f.where === "component:crashOne");
    check("a throwing component mount posts EXACTLY ONE ui_error", errs.length === 1, "count=" + errs.length);
    check("the ui_error carries where/instance/message", errs.length === 1 && errs[0].where === "component:crashOne" && errs[0].instance === "co1" && /mount-boom/.test(String(errs[0].message)), JSON.stringify(errs[0]));
  }
  // W2: rate limit — 5 rapid errors within the window yield ≤3 ui_error frames.
  advance(120000); // fresh window again
  win.eval("window.Biorouter.components.register('crashRate', { mount: function () { throw new Error('rate-boom'); } });");
  const baseW2 = received.length;
  emitUi({
    cmd: "render",
    target: "@region:results",
    body: [
      { t: "component", id: "cr1", name: "crashRate", props: {} },
      { t: "component", id: "cr2", name: "crashRate", props: {} },
      { t: "component", id: "cr3", name: "crashRate", props: {} },
      { t: "component", id: "cr4", name: "crashRate", props: {} },
      { t: "component", id: "cr5", name: "crashRate", props: {} },
    ],
  });
  await waitFor(() => doc.querySelectorAll(".br-unknown-widget[data-br-iid^='cr']").length === 5);
  await delay(60);
  {
    const errs = received.slice(baseW2).filter((f) => f.type === "ui_error" && f.where === "component:crashRate");
    check("5 rapid render errors are rate-limited to ≤3 ui_error frames", errs.length <= 3 && errs.length >= 1, "count=" + errs.length);
    check("all 5 failing instances still render a neutral placeholder", doc.querySelectorAll(".br-unknown-widget[data-br-iid^='cr']").length === 5);
  }
  // W3: after the window frees up, the next frame carries droppedCount for the drops.
  advance(180000); // window free again; 2 were dropped in W2
  win.eval("window.Biorouter.components.register('crashDrop', { mount: function () { throw new Error('drop-boom'); } });");
  const baseW3 = received.length;
  emitUi({ cmd: "render", target: "@region:results", body: [{ t: "component", id: "cd1", name: "crashDrop", props: {} }] });
  await waitFor(() => received.slice(baseW3).some((f) => f.type === "ui_error"));
  {
    const ue = received.slice(baseW3).find((f) => f.type === "ui_error");
    check("the next ui_error after drops carries droppedCount (=2)", !!ue && ue.droppedCount === 2, JSON.stringify(ue));
  }

  // ── Scenario X: multi-agent client facade (§3.8) ───────────────────────────
  // A `ready` frame advertising a worker profile ("critic") lets `br.agent()`
  // open a scoped facade whose prompts/calls carry `agent:critic`, whose `on()`
  // sees only critic frames, and whose activity is attributed in the presence
  // chip. Un-tagged (main) frames stay on the base client.
  send({
    type: "ready",
    protocol: 2,
    capabilities: ["ui"],
    sessionId: "harness",
    resumed: false,
    profiles: ["critic"],
  });
  await waitFor(() => win.eval("JSON.stringify(window.Biorouter.agents())") === '["critic"]');
  check(
    "ready.profiles is exposed via br.agents()",
    win.eval("JSON.stringify(window.Biorouter.agents())") === '["critic"]',
    win.eval("JSON.stringify(window.Biorouter.agents())")
  );

  // Start a scoped call and register a facade-scoped + a global listener.
  win.eval(
    "window.__critCall = null; window.__critMsgs = []; window.__mainMsgs = [];" +
      "window.Biorouter.agent('critic').on('message', function (ev) { window.__critMsgs.push(ev.delta); });" +
      "window.Biorouter.on('message', function (ev) { window.__mainMsgs.push(ev.delta); });" +
      "window.Biorouter.agent('critic').call({ text: 'grade this answer' }).then(function (r) { window.__critCall = r; });"
  );
  const baseAg = received.length;
  await waitFor(() => received.slice(baseAg).some((f) => f.type === "call" && f.agent === "critic"));
  const critCall = received.slice(baseAg).find((f) => f.type === "call" && f.agent === "critic");
  check(
    "br.agent(name).call sends a call frame carrying agent:name",
    !!critCall && critCall.agent === "critic" && critCall.text === "grade this answer",
    JSON.stringify(critCall)
  );

  // Two worker frames stream in; only the critic facade's on() should see them.
  send({ type: "message", delta: "Looks ", agent: "critic" });
  send({ type: "message", delta: "solid.", agent: "critic" });
  await waitFor(() => win.eval("window.__critMsgs.length") >= 2);
  check(
    "br.agent(name).on() receives ONLY that worker's frames",
    win.eval("JSON.stringify(window.__critMsgs)") === '["Looks ","solid."]',
    win.eval("JSON.stringify(window.__critMsgs)")
  );
  check(
    "a worker message frame shows the presence chip attributed to the profile",
    /^critic · /.test(String(win.eval("window.Biorouter.ui.presenceText() || ''"))),
    "presence=" + win.eval("String(window.Biorouter.ui.presenceText())")
  );

  // An un-tagged (main) frame reaches the global on() but NOT the critic facade.
  send({ type: "message", delta: "MAIN-ONLY" });
  await waitFor(() => win.eval("window.__mainMsgs.indexOf('MAIN-ONLY') >= 0"));
  check(
    "an un-tagged (main) frame never reaches a worker facade's on()",
    win.eval("window.__critMsgs.indexOf('MAIN-ONLY') < 0"),
    win.eval("JSON.stringify(window.__critMsgs)")
  );
  check(
    "the global on() still fires for ALL frames (main + worker, back-compat)",
    win.eval("window.__mainMsgs.indexOf('MAIN-ONLY') >= 0 && window.__mainMsgs.indexOf('Looks ') >= 0"),
    win.eval("JSON.stringify(window.__mainMsgs)")
  );

  // done{agent:critic} settles the critic call with its accumulated {text}.
  send({ type: "done", agent: "critic" });
  await waitFor(() => win.eval("window.__critCall") != null);
  check(
    "done with agent:critic settles the critic call with {text}",
    win.eval("window.__critCall && window.__critCall.text") === "Looks solid.",
    win.eval("JSON.stringify(window.__critCall)")
  );

  // Unknown profile → the async methods reject immediately, sending no frame.
  const baseGhost = received.length;
  win.eval(
    "window.__ghostCall = null; window.__ghostPrompt = null;" +
      "window.Biorouter.agent('ghost').call({ text: 'hi' }).then(function () { window.__ghostCall = 'ok'; }, function (e) { window.__ghostCall = String(e && e.message); });" +
      "window.Biorouter.agent('ghost').prompt('hi').then(function () { window.__ghostPrompt = 'ok'; }, function (e) { window.__ghostPrompt = String(e && e.message); });"
  );
  await waitFor(() => win.eval("window.__ghostCall") != null && win.eval("window.__ghostPrompt") != null);
  check(
    "an unknown agent profile's call rejects immediately with a clear error",
    /Unknown agent profile/i.test(String(win.eval("window.__ghostCall"))),
    win.eval("JSON.stringify(window.__ghostCall)")
  );
  check(
    "an unknown agent profile's prompt rejects immediately with a clear error",
    /Unknown agent profile/i.test(String(win.eval("window.__ghostPrompt"))),
    win.eval("JSON.stringify(window.__ghostPrompt)")
  );
  check(
    "an unknown-profile call/prompt sends no frame to the server",
    !received.slice(baseGhost).some((f) => f.agent === "ghost"),
    JSON.stringify(received.slice(baseGhost).map((f) => f.type + "/" + f.agent))
  );

  // ── Scenario Y: createApp config via a JSON <script> tag (CSP prep, Task 2) ──
  // A fresh page mounts the same bundle with a `#biorouter-app-config` JSON tag
  // (or a malformed one) and no working socket; we assert only on the config the
  // client latched. The WebSocket is stubbed so the probe never touches the live
  // daemon socket (config is read synchronously, before connect()).
  const TAG_ID = "biorouter-app-config";
  const jsonTag = (json) =>
    '<script type="application/json" id="' + TAG_ID + '">' + json + "</scr" + "ipt>";
  async function probeConfig(bodyHtml, windowConfig, htmlAttrs = "") {
    const doc2 =
      "<!doctype html><html" + (htmlAttrs ? " " + htmlAttrs : "") +
      "><head><title>cfg</title></head><body>" + bodyHtml + "</body></html>";
    const dom2 = new JSDOM(doc2, {
      url: `http://127.0.0.1:${PORT}/`,
      runScripts: "outside-only",
      pretendToBeVisual: true,
    });
    const w = dom2.window;
    const warns = [];
    // Stub WebSocket so the eager connect() fails fast and stays isolated.
    w.WebSocket = function () {
      throw new Error("no ws (config probe)");
    };
    w.console = {
      log: () => {},
      info: () => {},
      debug: () => {},
      error: () => {},
      warn: (...a) => warns.push(a.map(String).join(" ")),
    };
    if (windowConfig) w.BIOROUTER_APP_CONFIG = windowConfig;
    w.eval(bundle);
    await delay(30);
    const cfg = w.Biorouter ? w.Biorouter.config : null;
    const mode = w.document.documentElement.getAttribute("data-br-theme");
    const pack = w.document.documentElement.getAttribute("data-br-pack");
    dom2.window.close();
    return { cfg, warns, mode, pack };
  }

  const cfgTag = await probeConfig(jsonTag('{"appId":"cfg-from-tag","autoChat":false,"ui":false}'), null);
  check(
    "createApp reads config from a #biorouter-app-config JSON script tag",
    !!cfgTag.cfg && cfgTag.cfg.appId === "cfg-from-tag",
    JSON.stringify(cfgTag.cfg)
  );
  check(
    "the JSON script-tag config is fully applied (autoChat/ui carried through)",
    !!cfgTag.cfg && cfgTag.cfg.autoChat === false && cfgTag.cfg.ui === false,
    JSON.stringify(cfgTag.cfg)
  );

  const cfgPack = await probeConfig(
    jsonTag('{"appId":"cfg-pack","autoChat":false,"ui":false}'),
    null,
    'data-br-pack="midnight"'
  );
  check(
    "a persisted theme pack is not overwritten by the legacy light default",
    cfgPack.pack === "midnight" && cfgPack.mode === null,
    JSON.stringify({ pack: cfgPack.pack, mode: cfgPack.mode })
  );

  const cfgExplicitMode = await probeConfig(
    jsonTag('{"appId":"cfg-pack-mode","autoChat":false,"ui":false,"theme":"light"}'),
    null,
    'data-br-pack="midnight"'
  );
  check(
    "an explicit initial mode can still override a persisted theme pack",
    cfgExplicitMode.pack === "midnight" && cfgExplicitMode.mode === "light",
    JSON.stringify({ pack: cfgExplicitMode.pack, mode: cfgExplicitMode.mode })
  );

  const cfgPrec = await probeConfig(jsonTag('{"appId":"from-tag"}'), { appId: "from-window" });
  check(
    "the JSON script tag takes precedence over window.BIOROUTER_APP_CONFIG",
    !!cfgPrec.cfg && cfgPrec.cfg.appId === "from-tag",
    JSON.stringify(cfgPrec.cfg)
  );

  const cfgBad = await probeConfig(jsonTag("{ not valid json ]"), { appId: "cfg-from-window" });
  check(
    "malformed script-tag JSON falls back to window.BIOROUTER_APP_CONFIG",
    !!cfgBad.cfg && cfgBad.cfg.appId === "cfg-from-window",
    JSON.stringify(cfgBad.cfg)
  );
  check(
    "malformed script-tag JSON logs a warning",
    cfgBad.warns.some((wln) => /biorouter-app-config/i.test(wln)),
    JSON.stringify(cfgBad.warns)
  );

  // ── Scenario Z: unconsumed top-level frames degrade VISIBLY (issue #143) ────
  // The daemon sends `approval_refused` when an app page tries to grant a tool
  // approval it may not grant: the tool is DENIED and the turn moves on. Nothing
  // in the SDK consumed that frame, so the page said nothing and read as hung.
  // The general defect is worse than the one frame — an unknown TOP-LEVEL frame
  // reached `emit`, found an empty listener bucket and vanished.
  //
  // Driven in its own window because the timeline is mounted by `br.run()` into
  // the run target, and the main harness page has no run target of its own.
  const surfacesBefore = received.filter((f) => f.type === "ui_surface").length;
  const domZ = new JSDOM(
    '<!doctype html><html><head><title>Z</title></head><body><div id="tl"></div></body></html>',
    { url: `http://127.0.0.1:${PORT}/`, runScripts: "outside-only", pretendToBeVisual: true }
  );
  const winZ = domZ.window;
  const warnZ = [];
  winZ.WebSocket = globalThis.WebSocket;
  winZ.console = {
    log: () => {},
    info: () => {},
    debug: () => {},
    error: () => {},
    warn: (...a) => warnZ.push(a.map(String).join(" ")),
  };
  winZ.BIOROUTER_APP_CONFIG = { appId: APP_ID, autoChat: false, ui: true, wsToken: WS_TOKEN };
  winZ.eval(bundle);
  const upZ = await waitFor(
    () => received.filter((f) => f.type === "ui_surface").length > surfacesBefore
  );
  check("scenario Z page connects on its own socket", upZ, "no second ui_surface frame");

  // `br.run()` mounts a timeline into `.br-run-status` inside the target when the
  // app has no progress sink of its own. The run stays pending until we send
  // `done`, which is the window in which the frames below arrive.
  winZ.eval(
    "window.__z = null;" +
      "window.Biorouter.run('go', '#tl').then(function (t) { window.__z = t; }," +
      " function (e) { window.__z = 'ERR:' + e.message; });"
  );
  const mounted = await waitFor(() => !!winZ.document.querySelector("#tl .br-run-status"));
  check("br.run mounts a timeline into its target", mounted, "no .br-run-status in #tl");
  const zRows = () =>
    Array.from(winZ.document.querySelectorAll("#tl .br-run-step")).map((r) => r.textContent || "");

  const injectedTool = '<img id="injected-tool" src=x onerror="alert(1)">';
  send({ type: "tool", name: injectedTool, status: "running" });
  await waitFor(() => winZ.document.querySelector("#tl .br-run-answer")?.textContent.includes(injectedTool));
  check(
    "tool names render as text without injecting markup",
    !winZ.document.querySelector("#injected-tool") &&
      winZ.document.querySelector("#tl .br-run-answer")?.textContent.includes(injectedTool),
    winZ.document.querySelector("#tl .br-run-answer")?.innerHTML
  );

  // The `approval` frame is the control: it already had a row before this fix,
  // so a failure below is about `approval_refused`, not about the timeline.
  send({ type: "approval", requestId: "req-z1", tool: "developer__shell" });
  await waitFor(() => zRows().some((t) => /Approval needed/.test(t)));
  check(
    "an approval frame still draws its row (control)",
    zRows().some((t) => /Approval needed/.test(t)),
    JSON.stringify(zRows())
  );

  // ── The ORDINARY approval, which had no answer at all (issue #143's sibling)
  // The refused path above is the rare one. The common one is a tool the daemon
  // WOULD accept a decision for: `handle_action_required` parks the app socket's
  // only reader on it, so with no approve/reject affordance in the SDK the whole
  // connection froze behind a row that said "Approval needed" and offered no way
  // to answer it. These assertions are the affordance.
  const zActions = (id) =>
    winZ.document.querySelector('#tl [data-br-approval="' + id + '"]');
  const zButtons = (id) => {
    const host = zActions(id);
    return host ? Array.from(host.querySelectorAll("button")) : [];
  };
  // Null-safe on purpose: a regression that removes the controls entirely must
  // REPORT, not throw out of `check`'s argument list and abort the whole run.
  const zSettled = (id) => {
    const host = zActions(id);
    return host ? host.getAttribute("data-br-approval-settled") : null;
  };
  const zActionsHtml = (id) => (zActions(id) ? zActions(id).outerHTML : "(no controls)");
  check(
    "an approval row carries approve/reject buttons",
    zButtons("req-z1").map((b) => b.getAttribute("data-br-approval-action")).join(",") ===
      "approve,reject",
    JSON.stringify(zButtons("req-z1").map((b) => b.outerHTML))
  );

  send({ type: "approval_refused", requestId: "req-z1", reason: "unproven" });
  await waitFor(() => zRows().some((t) => /Approval refused/.test(t)));
  const refusedRow = zRows().find((t) => /Approval refused/.test(t)) || "";
  check(
    "approval_refused draws a visible row instead of vanishing",
    !!refusedRow,
    JSON.stringify(zRows())
  );
  check(
    "the approval_refused row says WHY and what it cost the run",
    /may not grant tool approvals/.test(refusedRow) && /the tool was denied/.test(refusedRow),
    refusedRow
  );
  check(
    "approval_refused is a KNOWN kind — never reported as unrecognized",
    !warnZ.some((w) => /unrecognized agent frame: approval_refused/i.test(w)) &&
      !zRows().some((t) => /Unrecognized event/.test(t)),
    JSON.stringify({ warns: warnZ, rows: zRows() })
  );
  check(
    "the approval_refused row is styled as an error, not as progress",
    !!Array.from(winZ.document.querySelectorAll("#tl .br-run-step--error")).find((r) =>
      /Approval refused/.test(r.textContent || "")
    ),
    JSON.stringify(
      Array.from(winZ.document.querySelectorAll("#tl .br-run-step")).map((r) => r.className)
    )
  );

  check(
    "a daemon-decided approval retires its own buttons",
    zSettled("req-z1") === "Refused" && zButtons("req-z1").length === 0,
    zActionsHtml("req-z1")
  );

  // Clicking Approve must reach the daemon as the `approve` frame
  // `handle_action_required` is parked reading. Nothing called `br.approve`
  // before, which is why the wait never ended.
  send({ type: "approval", requestId: "req-z2", tool: "developer__shell" });
  await waitFor(() => zButtons("req-z2").length === 2);
  (zButtons("req-z2")[0] || { dispatchEvent: () => {} }).dispatchEvent(new winZ.Event("click"));
  const approved = await waitFor(() =>
    received.some((f) => f.type === "approve" && f.request === "req-z2")
  );
  check(
    "clicking Approve sends the approve frame the daemon is waiting for",
    approved,
    JSON.stringify(received.filter((f) => f.type === "approve" || f.type === "reject"))
  );
  check(
    "the approved row settles and its buttons are gone (no second decision)",
    zSettled("req-z2") === "Approved" && zButtons("req-z2").length === 0,
    zActionsHtml("req-z2")
  );

  send({ type: "approval", requestId: "req-z3", tool: "developer__shell" });
  await waitFor(() => zButtons("req-z3").length === 2);
  (zButtons("req-z3")[1] || { dispatchEvent: () => {} }).dispatchEvent(new winZ.Event("click"));
  const rejected = await waitFor(() =>
    received.some((f) => f.type === "reject" && f.request === "req-z3")
  );
  check(
    "clicking Reject sends the reject frame",
    rejected,
    JSON.stringify(received.filter((f) => f.type === "approve" || f.type === "reject"))
  );
  check(
    "the rejected row settles too",
    zSettled("req-z3") === "Rejected" && zButtons("req-z3").length === 0,
    zActionsHtml("req-z3")
  );

  // The daemon's bounded wait giving up. It reuses `approval_refused` rather
  // than inventing a frame, so the row must say what a TIMEOUT means — a bare
  // "timeout" reads as a network fault rather than as a decision taken for you.
  send({ type: "approval", requestId: "req-z4", tool: "developer__shell" });
  await waitFor(() => zButtons("req-z4").length === 2);
  send({ type: "approval_refused", requestId: "req-z4", reason: "timeout" });
  await waitFor(() => zSettled("req-z4") === "Refused");
  const timeoutRow = zRows().find((t) => /nobody answered in time/.test(t)) || "";
  check(
    "an approval the daemon timed out says so in words, not as a bare reason code",
    /nobody answered in time/.test(timeoutRow) && /the tool was denied/.test(timeoutRow),
    JSON.stringify(zRows())
  );
  check(
    "a timed-out approval also retires its buttons",
    zSettled("req-z4") === "Refused" && zButtons("req-z4").length === 0,
    zActionsHtml("req-z4")
  );

  // The more important half: a frame type this SDK version has never heard of.
  const rowsBeforeUnknown = zRows().length;
  send({ type: "quantum_frame", requestId: "q1" });
  await waitFor(() => zRows().some((t) => /Unrecognized event/.test(t)));
  const unknownRow = zRows().find((t) => /Unrecognized event/.test(t)) || "";
  check(
    "an unknown top-level frame draws a visible row rather than vanishing",
    !!unknownRow,
    JSON.stringify(zRows())
  );
  check("the unrecognized row names the frame type", /quantum_frame/.test(unknownRow), unknownRow);
  check(
    "an unknown top-level frame is warned once",
    warnZ.filter((w) => /unrecognized agent frame: quantum_frame/i.test(w)).length === 1,
    JSON.stringify(warnZ)
  );
  send({ type: "quantum_frame", requestId: "q2" });
  await delay(120);
  check(
    "a repeat of the same unknown frame type does not re-warn",
    warnZ.filter((w) => /unrecognized agent frame: quantum_frame/i.test(w)).length === 1,
    JSON.stringify(warnZ)
  );
  send({ type: "tachyon_frame" });
  await waitFor(() => warnZ.some((w) => /tachyon_frame/i.test(w)));
  check(
    "a DIFFERENT unknown frame type warns again (once per type, not once ever)",
    warnZ.filter((w) => /unrecognized agent frame: tachyon_frame/i.test(w)).length === 1,
    JSON.stringify(warnZ)
  );

  // The other half of the default arm, and the one a careless fix breaks: a
  // KNOWN frame with no timeline row must still draw nothing. Treating "no
  // summary" as "unknown" would push a row per streamed delta.
  const rowsBeforeQuiet = zRows().length;
  send({ type: "thought", delta: "hmm" });
  send({ type: "usage", totalTokens: 12, model: "m" });
  send({ type: "message", delta: "partial answer" });
  await delay(150);
  check(
    "known-but-unsummarized frames (thought/usage/message) still draw no rows",
    zRows().length === rowsBeforeQuiet,
    `before=${rowsBeforeQuiet} after=${zRows().length} rows=${JSON.stringify(zRows())}`
  );
  check(
    "known-but-unsummarized frames are not warned about",
    !warnZ.some((w) => /unrecognized agent frame: (thought|usage|message)/i.test(w)),
    JSON.stringify(warnZ)
  );
  check(
    "the unknown-frame rows landed after the approval rows (ordering sanity)",
    zRows().length > rowsBeforeUnknown,
    `${rowsBeforeUnknown} → ${zRows().length}`
  );

  // The two other frames the daemon sends that had no consumer either.
  send({
    type: "capability_report",
    degraded: true,
    report: { missing_skills: ["variant-calling"], missing_knowledge_base: "cohort-kb" },
  });
  await waitFor(() => zRows().some((t) => /Capabilities missing/.test(t)));
  const capRow = zRows().find((t) => /Capabilities missing/.test(t)) || "";
  check(
    "capability_report draws a row naming what is missing",
    /variant-calling/.test(capRow) && /cohort-kb/.test(capRow),
    capRow
  );
  send({ type: "output_retry", attempt: 2, errors: ["not an object"] });
  await waitFor(() => zRows().some((t) => /Retrying structured output/.test(t)));
  check(
    "output_retry draws a row naming the attempt",
    /attempt 2/.test(zRows().find((t) => /Retrying structured output/.test(t)) || ""),
    JSON.stringify(zRows())
  );
  check(
    "neither capability_report nor output_retry is reported as unrecognized",
    !warnZ.some((w) => /unrecognized agent frame: (capability_report|output_retry)/i.test(w)),
    JSON.stringify(warnZ)
  );

  send({ type: "done" });
  await waitFor(() => winZ.eval("window.__z") !== null);
  check(
    "the run settles normally after all of the above",
    winZ.eval("window.__z") === "partial answer",
    JSON.stringify(winZ.eval("window.__z"))
  );

  const injectedError = '<img id="injected-error" src=x onerror="alert(1)">';
  winZ.__injectedError = injectedError;
  winZ.eval(
    "window.__promptBeforeError = window.Biorouter.prompt;" +
      "window.Biorouter.prompt = function () { return Promise.reject(new Error(window.__injectedError)); };" +
      "window.__errorFinished = false;" +
      "window.Biorouter.run('fail', '#tl', { progress: false }).catch(function () { window.__errorFinished = true; });"
  );
  await waitFor(() => winZ.__errorFinished);
  check(
    "run errors render as text without injecting markup",
    !winZ.document.querySelector("#injected-error") &&
      winZ.document.querySelector("#tl .br-run-answer")?.textContent.includes(injectedError),
    winZ.document.querySelector("#tl .br-run-answer")?.innerHTML
  );
  winZ.eval("window.Biorouter.prompt = window.__promptBeforeError;");
  domZ.window.close();

  // ── Scenario Z2: the known-kind list covers the AgentEvent union exactly ────
  // `KNOWN_EVENT_KINDS_MAP` is `Record<EventKind, true>`, so a missing or
  // misspelt key is a type error — for an editor or a `tsc` run. NOTHING in this
  // repo type-checks `sdk.ts` (esbuild transpiles, it does not check; the
  // no-esbuild fallback strips types outright), so the type alone is a claim, not
  // a gate. This is the gate: read the shipped source and compare the two lists.
  //
  // The direction that matters is ⊇ — a union member missing from the map turns
  // a REAL daemon frame into a red "Unrecognized event" row in every app, the
  // exact inverse of the bug the map exists to fix. ⊆ is checked too, because a
  // key for a frame the daemon does not send is a kind nothing can ever draw.
  const sdkSrc = await readFile(
    join(HERE, "../../crates/biorouter-mcp/src/agent_drafter/templates/sdk.ts"),
    "utf8"
  );
  const unionBlock = (sdkSrc.split("export type AgentEvent =")[1] || "").split(
    "\ntype EventKind"
  )[0];
  const unionKinds = [...unionBlock.matchAll(/type:\s*"([a-z_]+)"/g)].map((m) => m[1]);
  const mapBlock = (sdkSrc.split("const KNOWN_EVENT_KINDS_MAP")[1] || "").split("\n};")[0];
  const mapKinds = [...mapBlock.matchAll(/^\s{2}([a-z_]+):\s*true,$/gm)].map((m) => m[1]);
  check(
    "the AgentEvent union was parsed at all (guards a vacuous pass below)",
    unionKinds.length > 20 && mapKinds.length > 20,
    `union=${unionKinds.length} map=${mapKinds.length}`
  );
  const missingFromMap = unionKinds.filter((k) => mapKinds.indexOf(k) < 0);
  check(
    "every AgentEvent member is a KNOWN_EVENT_KINDS_MAP key",
    missingFromMap.length === 0,
    "missing: " + JSON.stringify(missingFromMap)
  );
  const strayInMap = mapKinds.filter((k) => unionKinds.indexOf(k) < 0);
  check(
    "the map names no kind the AgentEvent union does not",
    strayInMap.length === 0,
    "stray: " + JSON.stringify(strayInMap)
  );
  check(
    "the string list is DERIVED from the map, never written out twice",
    /const KNOWN_EVENT_KINDS:\s*string\[\]\s*=\s*Object\.keys\(KNOWN_EVENT_KINDS_MAP\)/.test(
      sdkSrc
    ),
    "KNOWN_EVENT_KINDS is not Object.keys(KNOWN_EVENT_KINDS_MAP)"
  );

  dom.window.close();
  log(failures === 0 ? "ALL PASS" : `${failures} FAILURE(S)`);
  return failures === 0 ? 0 : 1;
}
