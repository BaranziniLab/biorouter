// Dev server for the exported Biorouter app "prevent-ucsf-risk-calculator".
//
//   node serve.mjs          # installs the app, starts/reuses a daemon, serves + proxies
//   PORT=9000 node serve.mjs
//   BIOROUTERD_BIN=... BIOROUTERD_PORT=... node serve.mjs
//
// Static files come from this folder; anything under /apps/** (including the
// agent WebSocket at /apps/<id>/agent) is proxied to the Biorouter daemon, so
// the page and its agent share an origin and no port is ever hard-coded.
import { createServer, request as httpRequest } from "node:http";
import { spawn } from "node:child_process";
import { readFile, writeFile, rename, mkdir, cp, rm, access } from "node:fs/promises";
import { randomBytes } from "node:crypto";
import { constants } from "node:fs";
import { extname, normalize, resolve, join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";

const APP_ID = "prevent-ucsf-risk-calculator";
const PORT = Number(process.env.PORT || 8787);
const ROOT = dirname(fileURLToPath(import.meta.url));
const STORE = join(process.env.XDG_CONFIG_HOME || join(homedir(), ".config"),
                   "biorouter", "agent_drafter", APP_ID);
// A daemon serves an app's page only to a browser holding that app's access
// cookie, and the one-time link that sets it is handed only to a caller holding
// the daemon's secret. So this server uses a daemon it (or run.sh) started, whose
// port and secret are kept here readable by you alone, or one you name with
// BIOROUTERD_PORT and BIOROUTER_SERVER__SECRET_KEY, and no other.
const LAUNCHER_STATE = join(process.env.XDG_CONFIG_HOME || join(homedir(), ".config"),
                            "biorouter", "app-launcher", "daemon");
const MIME = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8", ".json": "application/json", ".svg": "image/svg+xml",
  ".png": "image/png", ".jpg": "image/jpeg", ".gif": "image/gif", ".webp": "image/webp",
  ".ico": "image/x-icon", ".woff2": "font/woff2", ".map": "application/json" };

const exists = (p) => access(p, constants.F_OK).then(() => true, () => false);

const DAEMON_PATHS = [
  process.env.BIOROUTERD_BIN,
  "biorouterd",
  join(homedir(), ".local/bin/biorouterd"),
  "/usr/local/bin/biorouterd",
  "/opt/homebrew/bin/biorouterd",
  "/Applications/Biorouter.app/Contents/Resources/bin/biorouterd",
  join(homedir(), "Applications/Biorouter.app/Contents/Resources/bin/biorouterd"),
  "/opt/Biorouter/resources/bin/biorouterd",
  // Linux GUI packages install under usr/lib/<name>: the deb maker lowercases
  // the name and the rpm maker preserves its case, so both spellings are real.
  "/usr/lib/biorouter/resources/bin/biorouterd",
  "/usr/lib/Biorouter/resources/bin/biorouterd",
].filter(Boolean);

/** Copy the runtime files into the store so the daemon can resolve the app. */
async function installApp() {
  if (!(await exists(join(ROOT, "manifest.json")))) {
    throw new Error("manifest.json missing from this folder - re-export the app.");
  }
  await mkdir(STORE, { recursive: true });
  await cp(join(ROOT, "manifest.json"), join(STORE, "manifest.json"));
  if (await exists(join(ROOT, "index.html"))) {
    await cp(join(ROOT, "index.html"), join(STORE, "index.html"));
  }
  for (const sub of ["src", "dist", "assets"]) {
    if (await exists(join(ROOT, sub))) {
      await rm(join(STORE, sub), { recursive: true, force: true });
      await cp(join(ROOT, sub), join(STORE, sub), { recursive: true });
    }
  }
}

function probe(port) {
  return new Promise((res) => {
    const r = httpRequest({ host: "127.0.0.1", port, path: "/status", timeout: 900 }, (resp) => {
      resp.resume();
      res(resp.statusCode === 200);
    });
    r.on("error", () => res(false));
    r.on("timeout", () => { r.destroy(); res(false); });
    r.end();
  });
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** One request to the daemon on `port` carrying its secret; resolves to { status, body }. */
function withSecret(port, secret, method, path) {
  return new Promise((res) => {
    const r = httpRequest(
      { host: "127.0.0.1", port, path, method, timeout: 5000, headers: { "X-Secret-Key": secret } },
      (resp) => {
        let body = "";
        resp.setEncoding("utf8");
        resp.on("data", (chunk) => { body += chunk; });
        resp.on("end", () => res({ status: resp.statusCode, body }));
      }
    );
    r.on("error", () => res({ status: 0, body: "" }));
    r.on("timeout", () => { r.destroy(); res({ status: 0, body: "" }); });
    r.end();
  });
}

const secretWorks = async (port, secret) =>
  (await withSecret(port, secret, "GET", "/apps")).status === 200;

async function rememberDaemon(port, secret) {
  try {
    await mkdir(dirname(LAUNCHER_STATE), { recursive: true, mode: 0o700 });
    const staged = `${LAUNCHER_STATE}.${process.pid}`;
    await writeFile(staged, `${port} ${secret}\n`, { mode: 0o600 });
    await rename(staged, LAUNCHER_STATE);
  } catch {
    console.warn("(could not remember this daemon; the next run starts another)");
  }
}

/** A daemon whose secret is known: named, remembered, or started here. */
async function ensureDaemon() {
  const named = process.env.BIOROUTER_SERVER__SECRET_KEY;
  const preferred = Number(process.env.BIOROUTERD_PORT || 3000);
  if (named && process.env.BIOROUTERD_PORT && (await secretWorks(preferred, named))) {
    console.log(`Using the Biorouter daemon on :${preferred}`);
    return { port: preferred, secret: named };
  }
  try {
    const [port, secret] = (await readFile(LAUNCHER_STATE, "utf8")).trim().split(/\s+/);
    if (port && secret && (await secretWorks(Number(port), secret))) {
      console.log(`Using the Biorouter daemon started for your apps on :${port}`);
      return { port: Number(port), secret };
    }
  } catch { /* no remembered daemon */ }
  for (const bin of DAEMON_PATHS) {
    for (const p of [preferred, 3001, 3002, 3003, 3004, 3005]) {
      // A daemon already here is not one whose secret this server knows.
      if (await probe(p)) continue;
      // A new secret for every attempt, so one asked of whatever held the port
      // never becomes the secret of a daemon started elsewhere.
      const secret = randomBytes(32).toString("hex");
      let child;
      try {
        child = spawn(bin, ["agent"], {
          env: { ...process.env, BIOROUTER_PORT: String(p), BIOROUTER_SERVER__SECRET_KEY: secret },
          stdio: "ignore", detached: true,
        });
      } catch { break; }          // this binary doesn't exist; try the next one
      let spawnFailed = false;
      child.on("error", () => { spawnFailed = true; });
      child.unref();
      for (let i = 0; i < 40 && !spawnFailed; i++) {
        if (await secretWorks(p, secret)) {
          console.log(`Started ${bin} on :${p}`);
          await rememberDaemon(p, secret);
          return { port: p, secret };
        }
        await sleep(500);
      }
      if (spawnFailed) break;
    }
  }
  throw new Error(
    "Could not find or start biorouterd. Install Biorouter, or set BIOROUTERD_BIN=/path/to/biorouterd"
  );
}

/** The path of this app's one-time launch link: opening it sets the app's access cookie. */
async function launchPath(port, secret) {
  const { status, body } = await withSecret(port, secret, "POST", `/apps/${APP_ID}/launch`);
  let path;
  try { path = JSON.parse(body).path; } catch { /* not JSON */ }
  if (status !== 200 || typeof path !== "string" || !/^\/apps\/[A-Za-z0-9_-]+\/\?t=[0-9a-f]{64}$/.test(path)) {
    throw new Error(`The daemon on :${port} does not serve '${APP_ID}'. Is the store at ${STORE}?`);
  }
  return path;
}

// Install the app, then bring up a daemon. On failure, print the actionable
// message (not an unhandled-rejection stack trace) and exit cleanly.
let daemonPort;
let launch;
try {
  await installApp();
  const daemon = await ensureDaemon();
  daemonPort = daemon.port;
  launch = await launchPath(daemon.port, daemon.secret);
} catch (e) {
  console.error("\n" + (e && e.message ? e.message : e) + "\n");
  process.exit(1);
}

/** Proxy an ordinary HTTP request to the daemon. */
function proxyHttp(req, res) {
  const up = httpRequest(
    { host: "127.0.0.1", port: daemonPort, path: req.url, method: req.method, headers: req.headers },
    (upRes) => {
      res.writeHead(upRes.statusCode || 502, upRes.headers);
      upRes.pipe(res);
    }
  );
  up.on("error", () => { res.writeHead(502); res.end("Biorouter daemon unreachable"); });
  req.pipe(up);
}

const server = createServer(async (req, res) => {
  const url = (req.url || "/").split("?")[0];
  if (url === "/apps" || url.startsWith("/apps/")) return proxyHttp(req, res);

  let path = decodeURIComponent(url);
  if (path === "/") path = "/index.html";
  // Resolve under ROOT; reject anything that escapes it (no path traversal).
  const file = resolve(ROOT, "." + normalize(path));
  if (file !== ROOT && !file.startsWith(ROOT + "/")) {
    res.writeHead(403); res.end("Forbidden"); return;
  }
  try {
    const body = await readFile(file);
    res.writeHead(200, { "Content-Type": MIME[extname(file)] || "application/octet-stream" });
    res.end(body);
  } catch {
    res.writeHead(404); res.end("Not found");
  }
});

// The agent WebSocket. Forward the upgrade verbatim and splice the sockets.
server.on("upgrade", (req, socket, head) => {
  const up = httpRequest({
    host: "127.0.0.1", port: daemonPort, path: req.url, method: "GET", headers: req.headers,
  });
  up.on("upgrade", (upRes, upSocket, upHead) => {
    const lines = ["HTTP/1.1 101 Switching Protocols"];
    for (const [k, v] of Object.entries(upRes.headers)) lines.push(`${k}: ${v}`);
    socket.write(lines.join("\r\n") + "\r\n\r\n");
    if (upHead && upHead.length) socket.write(upHead);
    upSocket.pipe(socket);
    socket.pipe(upSocket);
    const bail = () => { upSocket.destroy(); socket.destroy(); };
    upSocket.on("error", bail);
    socket.on("error", bail);
  });
  up.on("response", (r) => { socket.write(`HTTP/1.1 ${r.statusCode} \r\n\r\n`); socket.destroy(); });
  up.on("error", () => socket.destroy());
  if (head && head.length) up.write(head);
  up.end();
});

// Bind loopback ONLY. This server proxies the daemon's `/apps/**` routes and
// never adds the secret to what it forwards: a browser gets the app only with the
// access cookie its one-time launch link sets. Listening on 0.0.0.0 would still
// put the app's page one stolen link away from the whole LAN.
function listen(port, attempt = 0) {
  server.once("error", (e) => {
    if (e.code === "EADDRINUSE" && attempt < 10) {
      console.warn(`:${port} is busy, trying :${port + 1}`);
      listen(port + 1, attempt + 1);
    } else {
      console.error(e.message);
      process.exit(1);
    }
  });
  server.listen(port, "127.0.0.1", () => {
    console.log(`App running at http://127.0.0.1:${port}  (agent -> :${daemonPort})`);
    console.log(`Open it here (this address works once): http://127.0.0.1:${port}${launch}`);
  });
}
listen(PORT);
