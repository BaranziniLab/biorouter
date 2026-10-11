import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const source = readFileSync(
  join(repo, "crates/biorouter-mcp/src/agent_drafter/render.rs"),
  "utf8",
);
const template = source.match(
  /const RUN_PS1_TEMPLATE: &str = r##"([\s\S]*?)"##;/,
)?.[1];
assert(template, "RUN_PS1_TEMPLATE must exist");
function stringConstants(name) {
  const body = source.match(
    new RegExp(`pub const ${name}: &\\[&str\\] = &\\[([\\s\\S]*?)\\];`),
  )?.[1];
  assert(body, `${name} must exist`);
  return [...body.matchAll(/"([^"\\]*)"/g)].map((match) => match[1]);
}
const excluded = new Set([
  ...stringConstants("GENERATED_EXPORT_FILES"),
  ...stringConstants("RESERVED_EXPORT_DIRS"),
  "workspace",
  "node_modules",
]);
const launcher = template
  .replaceAll("__APP_ID__", "demo")
  .replaceAll("__EXPORT_DIGEST__", "")
  .replaceAll(
    "__SHIPPED_SKILLS__",
    stringConstants("SHIPPED_SKILL_NAMES").map(JSON.stringify).join(", "),
  )
  .replaceAll(
    "__RUNTIME_EXCLUDED__",
    [...excluded].map(JSON.stringify).join(", "),
  );
assert(
  launcher.includes("\nInstall-App\n"),
  "launcher installation boundary must exist",
);
const definitions = launcher.split("\nInstall-App\n")[0];
assert.equal(
  definitions.split('} catch { return "" }').length,
  2,
  "digest error boundary must exist exactly once",
);
const diagnostic = definitions.replace(
  '} catch { return "" }',
  "} catch { [Console]::Error.WriteLine($_.Exception.ToString()); [Console]::Error.WriteLine($_.ScriptStackTrace); throw }",
);
const variants = [
  ["actual-template", diagnostic],
  [
    "typed-array",
    diagnostic.replace(
      "$sorted = $lines.ToArray()",
      "[string[]]$sorted = $lines.ToArray()",
    ),
  ],
  [
    "typed-delegate",
    diagnostic.replace("::Create({", "::Create([Comparison[string]]{"),
  ],
  [
    "typed-array-and-delegate",
    diagnostic
      .replace(
        "$sorted = $lines.ToArray()",
        "[string[]]$sorted = $lines.ToArray()",
      )
      .replace("::Create({", "::Create([Comparison[string]]{"),
  ],
];

const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
const root = mkdtempSync(join(tmpdir(), "biorouter-ps-digest-"));
const command = process.env.BIOROUTER_TEST_POWERSHELL || "powershell.exe";
let failed = false;
try {
  for (const withMain of [true, false]) {
    const folder = join(root, `app-${withMain}`);
    const files = new Map([
      ["pages/index.html", Buffer.from("<html>entry</html>")],
      ["src/sdk.ts", Buffer.from("// sdk")],
      ["src/deep/a b.ts", Buffer.from("export const x = 1;\n")],
      ["styles.css", Buffer.from("body { color: navy; }")],
      ["scripts/chart.js", Buffer.from("draw();")],
      ["assets/\u{10000}.txt", Buffer.from("same content")],
      ["assets/\u{e000}.txt", Buffer.from("same content")],
      ["assets/logo.png", Buffer.from([0x89, 0x50, 0x4e, 0x47, 0, 0xff])],
      ["assets/.DS_Store", Buffer.from("finder")],
      ["dist/app.js", Buffer.from("// bundle")],
      ["workspace/data.csv", Buffer.from("runtime data")],
      ["manifest.json", Buffer.from("{}")],
    ]);
    if (withMain) files.set("src/main.ts", Buffer.from("import './sdk';"));
    for (const [name, bytes] of files) {
      const path = join(folder, ...name.split("/"));
      mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, bytes);
    }
    const lines = [...files]
      .filter(
        ([name]) =>
          !excluded.has(name.split("/")[0]) &&
          !name.endsWith("/.DS_Store") &&
          !(withMain && (name.startsWith("dist/") || name === "src/sdk.ts")),
      )
      .map(([name, bytes]) => Buffer.from(`${sha(bytes)} ${name}\n`, "utf8"))
      .sort(Buffer.compare);
    const expected = sha(Buffer.concat(lines));
    for (const [variant, body] of variants) {
      const probe = join(folder, `.digest-${variant}.ps1`);
      writeFileSync(
        probe,
        `${body}\n@{ powershell = $PSVersionTable.PSVersion.ToString(); digest = (Get-RuntimeDigest $Dir 'pages/index.html') } | ConvertTo-Json -Compress\n`,
        "utf8",
      );
      const output = spawnSync(
        command,
        [
          "-NoLogo",
          "-NoProfile",
          "-NonInteractive",
          "-ExecutionPolicy",
          "Bypass",
          "-File",
          probe,
        ],
        {
          env: { ...process.env, BIOROUTER_PATH_ROOT: join(root, "recipient") },
          encoding: "utf8",
          timeout: 15000,
          maxBuffer: 1024 * 1024,
        },
      );
      let result;
      try {
        result = JSON.parse(output.stdout);
      } catch {}
      const passed = output.status === 0 && result?.digest === expected;
      console.log(
        JSON.stringify({
          withMain,
          variant,
          passed,
          expected,
          actual: result?.digest,
          powershell: result?.powershell,
          status: output.status,
          error: output.error?.message,
        }),
      );
      if (!passed) {
        console.error(
          output.stderr ||
            output.stdout ||
            "PowerShell produced no diagnostic output",
        );
        if (variant === "actual-template") failed = true;
      }
    }
  }
} finally {
  rmSync(root, { recursive: true, force: true });
}
assert(
  !failed,
  "the actual Windows launcher digest must match UTF-8 byte ordering; variants above are diagnostics only",
);
