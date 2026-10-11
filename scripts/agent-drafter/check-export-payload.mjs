import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { createHash } from "node:crypto";
import {
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const source = readFileSync(
  join(repo, "crates/biorouter-mcp/src/agent_drafter/render.rs"),
  "utf8",
);
function constants(name) {
  const body = source.match(
    new RegExp(`pub const ${name}: &\\[&str\\] = &\\[([\\s\\S]*?)\\];`),
  )?.[1];
  assert(body, `${name} exists`);
  return [...body.matchAll(/"([^"\\]*)"/g)].map((match) => match[1]);
}
const shipped = constants("SHIPPED_SKILL_NAMES");
const excluded = [
  ...constants("GENERATED_EXPORT_FILES"),
  ...constants("RESERVED_EXPORT_DIRS"),
  "workspace",
  "node_modules",
];
function template(name, backend) {
  const body = source.match(
    new RegExp(`const ${name}: &str = r##"([\\s\\S]*?)"##;`),
  )?.[1];
  assert(body, `${name} exists`);
  return body
    .replaceAll("__APP_ID_JSON__", '"demo"')
    .replaceAll("__APP_ID__", "demo")
    .replaceAll("__EXPORT_DIGEST__", "")
    .replaceAll("__DEFAULT_PORT__", "8787")
    .replaceAll("__SHIPPED_SKILLS_JSON__", JSON.stringify(shipped))
    .replaceAll(
      "__SHIPPED_SKILLS__",
      backend === "bash"
        ? shipped.join("|")
        : shipped.map(JSON.stringify).join(", "),
    )
    .replaceAll(
      "__RUNTIME_EXCLUDED__",
      backend === "node"
        ? JSON.stringify(excluded)
        : excluded.map(JSON.stringify).join(", "),
    );
}
const bash = template("LAUNCHER_LIB_TEMPLATE", "bash");
const node = template("SERVE_MJS_TEMPLATE", "node").split(
  "// Install the app, then bring up a daemon.",
)[0];
const ps = template("RUN_PS1_TEMPLATE", "ps").split("\nInstall-App\n")[0];
assert(
  source.includes("install_app\nstart_daemon\ninstall_payload"),
  "Bash starts daemon before import",
);
assert(
  source.includes(
    '$Base = "http://127.0.0.1:$Port"\nInstall-Payload $Port $Secret',
  ),
  "PowerShell starts daemon before import",
);
assert(
  source.includes(
    "daemonPort = daemon.port;\n  await installPayload(daemon.port, daemon.secret);",
  ),
  "Node starts daemon before import",
);
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const secret = "export-payload-fixture-secret";
let calls = [];
let failSecond = false;
const allocations = new Map();
const server = createServer(async (req, res) => {
  if (req.headers["x-secret-key"] !== secret) {
    res.writeHead(401);
    res.end();
    return;
  }
  if (req.method === "GET" && req.url === "/apps") {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end("[]");
    return;
  }
  if (req.method !== "POST" || req.url !== "/knowledge/bases/import") {
    res.writeHead(404);
    res.end();
    return;
  }
  const chunks = [];
  for await (const chunk of req) chunks.push(chunk);
  const body = Buffer.concat(chunks).toString("utf8");
  const name = body.match(
    /name="?file"?;\s*filename="?([a-z0-9-]+)\.brkb"?/,
  )?.[1];
  assert(name, "multipart carries field file and valid archive filename");
  const bytes = body.split("\r\n\r\n")[1]?.split("\r\n--")[0];
  assert(
    bytes?.startsWith("archive:"),
    "actual archive bytes reached the endpoint",
  );
  calls.push({ name, bytes });
  if (name === "lab-second" && failSecond) {
    res.writeHead(500);
    res.end("fixture failure");
    return;
  }
  const ordinal = (allocations.get(name) || 1) + 1;
  allocations.set(name, ordinal);
  res.writeHead(200, { "Content-Type": "application/json" });
  res.end(JSON.stringify({ id: `${name}-${ordinal}` }));
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const port = server.address().port;
const aliasRoot = mkdtempSync(join(tmpdir(), "biorouter-export-payload-"));
const root = realpathSync.native(aliasRoot);
const backends =
  process.platform === "win32" ? ["node", "ps"] : ["bash", "node"];
if (process.env.BIOROUTER_TEST_POWERSHELL && !backends.includes("ps"))
  backends.push("ps");
const fingerprints = [];
function write(path, bytes) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, bytes);
}
async function execute(backend, folder, profile, yes = true) {
  let command, args;
  if (backend === "bash") {
    write(join(folder, "biorouter-launch.sh"), bash);
    write(
      join(folder, ".payload.sh"),
      "set -euo pipefail\n. ./biorouter-launch.sh\ninstall_app\nstart_daemon\ninstall_payload\n",
    );
    command = process.env.BIOROUTER_TEST_BASH || "/bin/bash";
    args = [join(folder, ".payload.sh")];
  } else if (backend === "node") {
    write(
      join(folder, ".payload.mjs"),
      `${node}\nawait installApp();\nconst daemon = await ensureDaemon();\nawait installPayload(daemon.port, daemon.secret);\nawait installApp();\n`,
    );
    command = process.execPath;
    args = [join(folder, ".payload.mjs")];
  } else {
    write(
      join(folder, ".payload.ps1"),
      `${ps}\nWrite-Host ('PAYLOAD_FIXTURE_STATE=' + (@{ powershell = $PSVersionTable.PSVersion.ToString(); dir = $Dir; sourceDigest = $SourceDigest; payloadState = $PayloadState } | ConvertTo-Json -Compress))\nInstall-App\n$daemon = Start-Daemon\nInstall-Payload ([int]$daemon[0]) ([string]$daemon[1])\n`,
    );
    command = process.env.BIOROUTER_TEST_POWERSHELL || "powershell.exe";
    args = [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      join(folder, ".payload.ps1"),
    ];
  }
  const env = {
    ...process.env,
    BIOROUTER_PATH_ROOT: profile,
    BIOROUTERD_PORT: String(port),
    BIOROUTER_SERVER__SECRET_KEY: secret,
  };
  delete env.BIOROUTER_EXPORT_REPLACE;
  if (yes) env.BIOROUTER_EXPORT_YES = "1";
  else delete env.BIOROUTER_EXPORT_YES;
  return await new Promise((done, reject) => {
    const child = spawn(command, args, {
      cwd: folder,
      env,
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stdout = "",
      stderr = "";
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error(`${backend} fixture timed out`));
    }, 30000);
    child.on("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      assert.equal(code, 0, `${backend}: ${stdout}\n${stderr}`);
      assert(
        !stdout.includes(secret) && !stderr.includes(secret),
        "secret stays out of diagnostics",
      );
      done(stdout + stderr);
    });
    child.stdin.end(yes ? "" : "n\n");
  });
}
function prepare(name) {
  const folder = join(root, name, "My Apps", "demo");
  const profile = join(root, name, "recipient");
  const manifest = {
    id: "demo",
    title: "Demo",
    kind: "agentic",
    entry: "index.html",
    created_at: 100,
    updated_at: 200,
    app_uid: "payload-fixture",
    agent: {
      knowledge_base: "lab-reference",
      system_prompt: "Preserve lab-reference text",
      requires: [
        {
          kind: "knowledge_base",
          id: "lab-reference",
          reason: "Keep lab-reference reason",
        },
        { kind: "knowledge_base", id: "lab-second" },
        { kind: "skill", id: "lab-reference" },
      ],
    },
  };
  write(
    join(folder, "manifest.json"),
    JSON.stringify(manifest, null, 2) + "\n",
  );
  write(join(folder, "index.html"), "<html>payload app</html>");
  write(join(folder, "export.json"), '{"mode":"full"}\n');
  write(
    join(folder, "payload/knowledge/lab-reference.brkb"),
    "archive:reference-v1",
  );
  write(join(folder, "payload/knowledge/lab-second.brkb"), "archive:second-v1");
  write(join(folder, "payload/skills/lab-units/SKILL.md"), "exported skill");
  write(join(folder, "payload/skills/Office-Word/SKILL.md"), "must skip");
  write(join(profile, "config/skills/lab-units/SKILL.md"), "recipient skill");
  write(join(profile, "config/skills/Office-Word/SKILL.md"), "seeded skill");
  write(
    join(profile, "config/.apps-payload-installed-demo"),
    "old staging-only marker",
  );
  return {
    folder,
    profile,
    manifest,
    store: join(profile, "config/agent_drafter/demo/manifest.json"),
    state: join(
      profile,
      "config/app-launcher/payload/demo",
      digest(Buffer.from(realpathSync.native(folder))),
    ),
  };
}
try {
  for (const backend of backends) {
    calls = [];
    allocations.clear();
    failSecond = true;
    const test = prepare(backend);
    const { folder, profile, store, state, manifest } = test;
    const sourceBefore = readFileSync(join(folder, "manifest.json"), "utf8");
    const failed = await execute(backend, folder, profile);
    assert(
      failed.includes("not fully installed"),
      `${backend} reports partial failure: ${failed}`,
    );
    const stateDiagnostic = failed
      .split(/\r?\n/)
      .find((line) => line.startsWith("PAYLOAD_FIXTURE_STATE="));
    if (stateDiagnostic) console.log(stateDiagnostic);
    assert(
      existsSync(state),
      `${backend} expected payload state ${state}; actual source-folder records: ${existsSync(dirname(state)) ? readdirSync(dirname(state)).join(", ") : "none"}; launcher output:\n${failed}`,
    );
    assert(
      !readdirSync(state).includes("complete"),
      `${backend} no completion on failure`,
    );
    assert.equal(
      readdirSync(join(state, "knowledge")).length,
      1,
      `${backend} successful first receipt retained`,
    );
    assert.equal(
      readFileSync(join(profile, "config/skills/lab-units/SKILL.md"), "utf8"),
      "recipient skill",
      `${backend} failed bases do not repeatedly install skills`,
    );
    failSecond = false;
    const succeeded = await execute(backend, folder, profile);
    assert(
      succeeded.includes("Payload installed."),
      `${backend} retry completes: ${succeeded}`,
    );
    assert.equal(
      calls.filter((call) => call.name === "lab-reference").length,
      1,
      `${backend} successful base is not imported twice`,
    );
    let current = JSON.parse(readFileSync(store, "utf8"));
    assert.equal(
      current.agent.knowledge_base,
      "lab-reference-2",
      `${backend} collision binding`,
    );
    assert.equal(
      current.agent.requires[0].id,
      "lab-reference-2",
      `${backend} collision requirement`,
    );
    assert.equal(
      current.agent.requires[1].id,
      "lab-second-2",
      `${backend} second requirement`,
    );
    assert.equal(
      current.agent.requires[2].id,
      "lab-reference",
      `${backend} skill requirement untouched`,
    );
    assert.equal(current.agent.system_prompt, manifest.agent.system_prompt);
    assert.equal(
      current.agent.requires[0].reason,
      manifest.agent.requires[0].reason,
    );
    assert.equal(
      readFileSync(join(profile, "config/skills/Office-Word/SKILL.md"), "utf8"),
      "seeded skill",
    );
    assert.equal(
      readFileSync(join(profile, "config/skills/lab-units/SKILL.md"), "utf8"),
      "exported skill",
    );
    const backup = readdirSync(join(profile, "config/app-launcher/backups"))[0];
    assert.equal(
      readFileSync(
        join(
          profile,
          "config/app-launcher/backups",
          backup,
          "lab-units/SKILL.md",
        ),
        "utf8",
      ),
      "recipient skill",
    );
    const count = calls.length;
    await execute(backend, folder, profile);
    assert.equal(calls.length, count, `${backend} replay does not import`);
    if (
      aliasRoot !== root &&
      (backend !== "ps" || process.platform === "win32")
    ) {
      const aliasFolder = join(aliasRoot, relative(root, folder));
      const aliasProfile = join(aliasRoot, relative(root, profile));
      await execute(backend, aliasFolder, aliasProfile);
      assert.equal(
        calls.length,
        count,
        `${backend} an alternate spelling of the same source folder reuses knowledge receipts`,
      );
      console.log(`${backend}: source-folder alias reuses payload receipts`);
      if (backend === "ps" && process.platform === "win32") {
        await execute("node", aliasFolder, aliasProfile);
        assert.equal(
          calls.length,
          count,
          "Node reuses PowerShell knowledge receipts through the Windows short path alias",
        );
        console.log(
          "Windows PowerShell and Node share payload receipts through the short path alias",
        );
      }
    }
    // An installation that recopies the export's manifest must recover recipient IDs.
    write(store, sourceBefore);
    await execute(backend, folder, profile);
    current = JSON.parse(readFileSync(store, "utf8"));
    assert.equal(
      current.agent.knowledge_base,
      "lab-reference-2",
      `${backend} restore mapping after manifest copy`,
    );
    current.agent.knowledge_base = "independent-base";
    current.agent.requires[0].id = "independent-base";
    current.agent.system_prompt = "recipient edit";
    write(store, JSON.stringify(current, null, 2) + "\n");
    await execute(backend, folder, profile);
    current = JSON.parse(readFileSync(store, "utf8"));
    assert.equal(current.agent.knowledge_base, "independent-base");
    assert.equal(current.agent.requires[0].id, "independent-base");
    assert.equal(current.agent.system_prompt, "recipient edit");
    current.agent.knowledge_base = "lab-reference-2";
    current.agent.requires[0].id = "lab-reference-2";
    write(store, JSON.stringify(current, null, 2) + "\n");
    write(
      join(folder, "payload/knowledge/lab-reference.brkb"),
      "archive:reference-v2",
    );
    await execute(backend, folder, profile);
    current = JSON.parse(readFileSync(store, "utf8"));
    assert.equal(
      current.agent.knowledge_base,
      "lab-reference-3",
      `${backend} changed archive replaces prior allocated binding`,
    );
    assert.equal(current.agent.requires[0].id, "lab-reference-3");
    assert.equal(current.agent.system_prompt, "recipient edit");
    assert.equal(
      readFileSync(join(folder, "manifest.json"), "utf8"),
      sourceBefore,
      `${backend} original exporter is preserved`,
    );
    fingerprints.push(readFileSync(join(state, "complete"), "utf8").trim());
    const declined = prepare(`${backend}-decline`);
    const before = calls.length;
    const output = await execute(
      backend,
      declined.folder,
      declined.profile,
      false,
    );
    assert(output.includes("Skipping payload install"));
    assert.equal(calls.length, before, `${backend} denial sends no import`);
    const chained = prepare(`${backend}-chain`);
    const chainManifest = JSON.parse(
      readFileSync(join(chained.folder, "manifest.json"), "utf8"),
    );
    chainManifest.agent.requires[1].id = "lab-reference-2";
    write(
      join(chained.folder, "manifest.json"),
      JSON.stringify(chainManifest, null, 2) + "\n",
    );
    rmSync(join(chained.folder, "payload/knowledge/lab-second.brkb"));
    write(
      join(chained.folder, "payload/knowledge/lab-reference-2.brkb"),
      "archive:chained",
    );
    allocations.clear();
    await execute(backend, chained.folder, chained.profile);
    let chainCurrent = JSON.parse(readFileSync(chained.store, "utf8"));
    assert.equal(
      chainCurrent.agent.knowledge_base,
      "lab-reference-2",
      `${backend} allocated ID cannot cascade into another source ID`,
    );
    assert.equal(chainCurrent.agent.requires[0].id, "lab-reference-2");
    assert.equal(chainCurrent.agent.requires[1].id, "lab-reference-2-2");
    write(chained.store, JSON.stringify(chainManifest, null, 2) + "\n");
    await execute(backend, chained.folder, chained.profile);
    chainCurrent = JSON.parse(readFileSync(chained.store, "utf8"));
    assert.equal(chainCurrent.agent.knowledge_base, "lab-reference-2");
    assert.equal(chainCurrent.agent.requires[1].id, "lab-reference-2-2");
    console.log(
      `${backend}: authenticated import, collision mapping, retries, receipts, consent, skills, and source preservation pass`,
    );
  }
  assert.equal(
    new Set(fingerprints).size,
    1,
    "payload fingerprint matches across launcher languages",
  );
} finally {
  await new Promise((done) => server.close(done));
  rmSync(root, { recursive: true, force: true });
}
