// Send a cohort (or one patient) from a Biorouter chat to the PREVENT UCSF calculator.
//
//   node handoff.mjs cohort  <rows.csv|rows.json> --name "..." --purpose "..." --definition "..." --as-of 2016-01-01 [--counts '{"cohort_size": 10977}']
//   node handoff.mjs patient <one-row.csv|.json>  [--key C0017] [--sources '{"sbp": {"source": "OMOP measurement", "date": "2015-11-02"}}']
//   options: --source "..."  --port 3000 (calculator daemon; found automatically)  --keep-file  --no-open  --timeout 120
//
// Required columns (any order): age, sex, sbp, total_cholesterol_mg_dl, hdl_mg_dl, bmi, egfr, diabetes,
// current_smoker, bp_treatment, statin. Accepted aliases: total_cholesterol, hdl, smoking, female (1 = female).
// Identifier columns (person_id, *_id, mrn) are dropped and never sent; members get keys C00001, C00002, ...
// The rows file is deleted after reading unless --keep-file is given.
//
// How the data travel: this script opens a one-time WebSocket on 127.0.0.1 and opens the calculator with a
// pointer to it (#handoff=ws://127.0.0.1:<port>/<token>). The calculator connects, receives the data once, and
// the socket closes. The data never appear in the address, the browser history or this script's output.
// The calculator must already be open in the browser (run.sh), because its page needs the access cookie.
import { createServer, request } from "node:http";
import { createHash, randomBytes } from "node:crypto";
import { readFileSync, unlinkSync } from "node:fs";
import { spawn } from "node:child_process";

const APP_ID = "prevent-ucsf-risk-calculator";
const INPUTS = ["age", "sex", "sbp", "total_cholesterol_mg_dl", "hdl_mg_dl", "bmi", "egfr", "diabetes", "current_smoker", "bp_treatment", "statin"];
const ALIASES = { total_cholesterol: "total_cholesterol_mg_dl", hdl: "hdl_mg_dl", smoking: "current_smoker", smoker: "current_smoker", bp_tx: "bp_treatment" };
const ID_COLUMN = /^(person_id|patient_id|subject_id|mrn|.*_id)$/i;

function die(msg, code = 1) { console.error("handoff: " + msg); process.exit(code); }
const argv = process.argv.slice(2);
const kind = argv[0]; const file = argv[1];
if (!["cohort", "patient"].includes(kind) || !file) die("usage: node handoff.mjs cohort|patient <rows.csv|rows.json> [options]");
const opt = {}; for (let i = 2; i < argv.length; i++) { const a = argv[i]; if (!a.startsWith("--")) die("unexpected argument " + a); const k = a.slice(2); if (["keep-file", "no-open"].includes(k)) opt[k] = true; else opt[k] = argv[++i]; }

// ── read rows ──
function parseCsv(text) {
  const rows = []; let row = [], cur = "", q = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (q) { if (c === '"') { if (text[i + 1] === '"') { cur += '"'; i++; } else q = false; } else cur += c; }
    else if (c === '"') q = true; else if (c === ",") { row.push(cur); cur = ""; }
    else if (c === "\n" || c === "\r") { if (c === "\r" && text[i + 1] === "\n") i++; row.push(cur); cur = ""; if (row.some((x) => x !== "")) rows.push(row); row = []; }
    else cur += c;
  }
  row.push(cur); if (row.some((x) => x !== "")) rows.push(row);
  const head = rows.shift() || []; return { head: head.map((h) => h.trim()), rows };
}
let head, rows;
let text; try { text = readFileSync(file, "utf8"); } catch (e) { die("cannot read " + file + ": " + e.message); }
if (/\.json$/i.test(file)) {
  const j = JSON.parse(text);
  if (Array.isArray(j)) { head = Object.keys(j[0] || {}); rows = j.map((o) => head.map((h) => o[h])); }
  else if (j && Array.isArray(j.columns) && Array.isArray(j.rows)) { head = j.columns; rows = j.rows; }
  else die("JSON must be an array of objects or {columns, rows}");
} else ({ head, rows } = parseCsv(text));
if (!opt["keep-file"]) { try { unlinkSync(file); } catch { /* already gone */ } }

const dropped = head.filter((h) => ID_COLUMN.test(h));
const col = {}; head.forEach((h, i) => { if (!ID_COLUMN.test(h)) col[ALIASES[h.toLowerCase()] || h.toLowerCase()] = i; });
if (!("sex" in col) && "female" in col) col.__female = col.female;
const missingCols = INPUTS.filter((k) => !(k in col) && !(k === "sex" && "__female" in col));
if (missingCols.length) die("missing column(s): " + missingCols.join(", "));
const out = rows.map((r, i) => ["C" + String(i + 1).padStart(5, "0")].concat(INPUTS.map((k) => {
  if (k === "sex" && !("sex" in col)) { const f = String(r[col.__female] ?? "").trim(); return f === "1" || f.toLowerCase() === "true" ? "female" : f === "" ? "" : "male"; }
  const v = r[col[k]]; return v === undefined || v === null ? "" : v;
})));
if (kind === "patient" && out.length !== 1) die("patient mode needs exactly one row (found " + out.length + ")");
if (out.length > 50000) die("too many rows (" + out.length + "); send at most 50,000");

let payload;
if (kind === "cohort") {
  let counts = {}; if (opt.counts) { try { counts = JSON.parse(opt.counts); } catch { die("--counts must be JSON"); } }
  payload = { v: 1, kind, name: opt.name || "Cohort from Biorouter chat", purpose: opt.purpose || "", definition: opt.definition || "", as_of: opt["as-of"] || "", source: opt.source || "UCSF OMOP via Biorouter chat", counts, columns: ["key"].concat(INPUTS), rows: out };
} else {
  let sources = {}; if (opt.sources) { try { sources = JSON.parse(opt.sources); } catch { die("--sources must be JSON"); } }
  const inputs = {}; INPUTS.forEach((k, i) => { inputs[k] = out[0][i + 1]; });
  payload = { v: 1, kind, key: opt.key || "", source: opt.source || "UCSF OMOP via Biorouter chat", sources, inputs };
}
const body = Buffer.from(JSON.stringify(payload));

// ── find the calculator's daemon: it answers 401 (cookie needed) on the app path ──
function status(port) {
  return new Promise((resolve) => {
    const rq = request({ host: "127.0.0.1", port, path: "/apps/" + APP_ID + "/", method: "GET", timeout: 800 }, (res) => { res.resume(); resolve(res.statusCode); });
    rq.on("error", () => resolve(0)); rq.on("timeout", () => { rq.destroy(); resolve(0); }); rq.end();
  });
}
let appPort = opt.port ? Number(opt.port) : 0;
if (!appPort) for (let p = 3000; p <= 3005; p++) { if ((await status(p)) === 401) { appPort = p; break; } }
if (!appPort) die("the calculator is not running: open it with run.sh, then run this again", 2);

// ── one-time WebSocket server ──
const token = randomBytes(24).toString("hex");
const allowedOrigins = new Set(["http://127.0.0.1:" + appPort, "http://localhost:" + appPort]);
function frame(buf, opcode) {
  const len = buf.length; let headLen = len < 126 ? 2 : len < 65536 ? 4 : 10; const h = Buffer.alloc(headLen);
  h[0] = 0x80 | opcode;
  if (len < 126) h[1] = len; else if (len < 65536) { h[1] = 126; h.writeUInt16BE(len, 2); } else { h[1] = 127; h.writeBigUInt64BE(BigInt(len), 2); }
  return Buffer.concat([h, buf]);
}
let delivered = false;
const server = createServer((_, res) => { res.writeHead(404); res.end(); });
server.on("upgrade", (req, socket) => {
  if (delivered || req.url !== "/" + token || !allowedOrigins.has(String(req.headers.origin || "")) || !req.headers["sec-websocket-key"]) { socket.destroy(); return; }
  const accept = createHash("sha1").update(req.headers["sec-websocket-key"] + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest("base64");
  socket.write("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: " + accept + "\r\n\r\n");
  socket.write(frame(body, 0x1)); socket.write(frame(Buffer.alloc(0), 0x8)); socket.end();
  delivered = true;
  const n = kind === "cohort" ? out.length + " members" : "1 patient";
  console.log("Delivered " + n + " to the PREVENT calculator" + (dropped.length ? " (identifier columns dropped: " + dropped.join(", ") + ")" : "") + ".");
  setTimeout(() => process.exit(0), 200);
});
server.listen(0, "127.0.0.1", () => {
  const wsPort = server.address().port;
  const url = "http://127.0.0.1:" + appPort + "/apps/" + APP_ID + "/#handoff=ws://127.0.0.1:" + wsPort + "/" + token;
  console.log("Ready: " + (kind === "cohort" ? out.length + " rows" : "1 patient") + ", " + INPUTS.length + " inputs" + (dropped.length ? "; identifier columns dropped: " + dropped.join(", ") : "") + ".");
  if (opt["no-open"]) console.log("Open in the browser where the calculator is open: " + url);
  else {
    const opener = process.platform === "darwin" ? "open" : process.platform === "win32" ? "explorer" : "xdg-open";
    spawn(opener, [url], { stdio: "ignore", detached: true }).unref();
    console.log("Opening the calculator in the default browser…");
  }
});
const timeout = Number(opt.timeout || 120) * 1000;
setTimeout(() => { if (!delivered) die("not received within " + timeout / 1000 + " s: is the calculator open in this browser? Open it with run.sh, then run this again", 3); }, timeout);
