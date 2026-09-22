/**
 * Two halves, and both are necessary.
 *
 * FIRST, the instrument is checked against fixtures — including the cases that
 * would make it pass when it should not (a `windowsHide` that is only a comment,
 * only a string, or nested one level down). A census nobody has watched FAIL is
 * a green light wired to nothing, which is the failure mode this whole area has
 * already produced once.
 *
 * SECOND, the real tree is walked, with floors under the walk. A walk that
 * silently reads nothing reports zero violations and passes; the floors turn
 * that into a failure.
 */
import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import {
  SRC_ROOT,
  VISIBLE_BY_DESIGN,
  auditTree,
  collectSpawnSites,
} from './console-window-census.mjs';

const statesOf = (source) => collectSpawnSites(source).map((s) => `${s.callee}:${s.state}`);

// ── The instrument, proven able to fail ──────────────────────────────────────

test('a spawn with no options at all is a violation', () => {
  assert.deepEqual(
    statesOf(`import { spawn } from 'child_process';\nspawn('git', ['status']);\n`),
    ['spawn:no-options']
  );
});

test('an options object that omits windowsHide is a violation', () => {
  assert.deepEqual(
    statesOf(`import { spawn } from 'node:child_process';\nspawn('git', [], { shell: false });\n`),
    ['spawn:missing']
  );
});

test('windowsHide: true is accepted', () => {
  assert.deepEqual(
    statesOf(`import { spawn } from 'child_process';\nspawn('git', [], { windowsHide: true });\n`),
    ['spawn:hidden']
  );
});

test('windowsHide: false is reported separately, not silently accepted', () => {
  assert.deepEqual(
    statesOf(`import { spawn } from 'child_process';\nspawn('git', [], { windowsHide: false });\n`),
    ['spawn:visible']
  );
});

test('a windowsHide that is only a comment does not count', () => {
  assert.deepEqual(
    statesOf(
      `import { spawn } from 'child_process';\n// windowsHide: true\nspawn('git', [], { shell: false });\n`
    ),
    ['spawn:missing']
  );
});

test('a windowsHide that is only a string does not count', () => {
  assert.deepEqual(
    statesOf(
      `import { spawn } from 'child_process';\nconst note = 'windowsHide: true';\nspawn('git', [], { shell: false });\n`
    ),
    ['spawn:missing']
  );
});

test('a windowsHide nested inside another option does not count', () => {
  assert.deepEqual(
    statesOf(
      `import { spawn } from 'child_process';\nspawn('git', [], { env: { windowsHide: true } });\n`
    ),
    ['spawn:missing']
  );
});

test('a non-literal windowsHide is reported rather than assumed true', () => {
  assert.deepEqual(
    statesOf(
      `import { spawn } from 'child_process';\nconst hide = true;\nspawn('git', [], { windowsHide: hide });\n`
    ),
    ['spawn:unknown']
  );
});

test('an aliased named import is still a spawn site', () => {
  assert.deepEqual(statesOf(`import { spawn as sp } from 'child_process';\nsp('git', []);\n`), [
    'sp:no-options',
  ]);
});

test('a namespace import is still a spawn site', () => {
  assert.deepEqual(
    statesOf(`import * as cp from 'node:child_process';\ncp.execFile('git', []);\n`),
    ['cp.execFile:no-options']
  );
});

test('require() in both shapes is still a spawn site', () => {
  assert.deepEqual(statesOf(`const { spawn } = require('child_process');\nspawn('git');\n`), [
    'spawn:no-options',
  ]);
  assert.deepEqual(statesOf(`const cp = require('child_process');\ncp.spawn('git');\n`), [
    'cp.spawn:no-options',
  ]);
});

test('promisify(execFile) is followed, because that is how the app reaches it', () => {
  assert.deepEqual(
    statesOf(
      `import { execFile } from 'node:child_process';\nimport { promisify } from 'node:util';\nconst run = promisify(execFile);\nawait run('sips', ['-g'], { timeout: 10 });\n`
    ),
    ['run:missing']
  );
});

test('an options object held in a const is resolved', () => {
  assert.deepEqual(
    statesOf(
      `import { spawn } from 'child_process';\nconst opts = { stdio: 'pipe', windowsHide: true };\nspawn('git', [], opts);\n`
    ),
    ['spawn:hidden']
  );
});

test('a callback argument does not hide the options object', () => {
  assert.deepEqual(
    statesOf(
      `import { execFile } from 'child_process';\nexecFile('git', [], { windowsHide: true }, (e) => e);\n`
    ),
    ['execFile:hidden']
  );
});

test('a local function that merely shares the name is not a spawn site', () => {
  assert.deepEqual(statesOf(`function spawn(a) { return a; }\nspawn('not a process');\n`), []);
});

test('a spawn imported from somewhere else is not a spawn site', () => {
  assert.deepEqual(statesOf(`import { spawn } from './myPool';\nspawn('worker');\n`), []);
});

// ── The real tree ────────────────────────────────────────────────────────────

const AUDIT = auditTree();

test('every production spawn site states windowsHide', () => {
  assert.equal(
    AUDIT.violations.length,
    0,
    `\n${AUDIT.violations
      .map((v) => `  ${v.file}:${v.line} ${v.callee}(…) — ${v.why}`)
      .join(
        '\n'
      )}\n\nOn Windows the Electron main process owns no console, so a console-subsystem` +
      ' child spawned without windowsHide gets a NEW, VISIBLE one. Add `windowsHide: true`, or' +
      ' `windowsHide: false` plus a row in VISIBLE_BY_DESIGN saying why a window is wanted.'
  );
});

test('the walk actually read the tree', () => {
  // Floors, not equalities: a new component must not fail this, but a walk that
  // stops early must. Measured 2026-09-22: 644 files, 19 sites, 1 pty site.
  assert.ok(AUDIT.files >= 500, `only ${AUDIT.files} production files walked`);
  assert.ok(AUDIT.sites.length >= 18, `only ${AUDIT.sites.length} spawn sites found`);
});

test('the known spawning files are all found', () => {
  // Named individually, so a walk that reads most of the tree but misses the one
  // file that matters still fails. These counts are minimums.
  const expected = [
    ['src/biorouterd.ts', 2],
    ['src/main.ts', 9],
    ['src/utils/artifactGit.ts', 1],
    ['src/utils/dependencyChecker.ts', 4],
    ['src/utils/heicConvert.ts', 2],
  ];
  for (const [file, atLeast] of expected) {
    const found = AUDIT.sites.filter((s) => s.file === file).length;
    assert.ok(
      found >= atLeast,
      `${file}: found ${found} spawn sites, expected at least ${atLeast}`
    );
  }
});

test('every VISIBLE_BY_DESIGN row still matches a real site, and carries a reason', () => {
  // An allowlist row that no longer matches anything is dead permission: it
  // stops describing the tree and starts hiding whatever moves into its place.
  for (const row of VISIBLE_BY_DESIGN) {
    assert.ok(row.reason.length > 40, `VISIBLE_BY_DESIGN row for ${row.file} needs a real reason`);
    const matched = AUDIT.sites.filter(
      (s) => s.file.endsWith(row.file) && s.text.includes(row.match) && s.state === 'visible'
    );
    assert.equal(
      matched.length,
      1,
      `VISIBLE_BY_DESIGN row ${row.file} / ${row.match} matched ${matched.length} sites`
    );
  }
});

test('node-pty is spawned from exactly one place, and it is the terminal', () => {
  // node-pty is not child_process, so nothing above covers it. On Windows it
  // goes through ConPTY, which runs `conhost.exe --headless` and shows no
  // window; and it only runs when the user opens the terminal dock. Both of
  // those are properties of ONE call site, so this pins the count.
  assert.equal(AUDIT.ptySites.length, 1, 'node-pty is spawned from more than one place now');
  assert.equal(AUDIT.ptySites[0].file, 'src/main.ts');
});

test('the Windows behavioural test still exists and still has its control', () => {
  // This census only proves the option is WRITTEN. The proof that it WORKS runs
  // on Windows, and is worthless without its control — so the control is pinned
  // from here, where it is checked on every platform.
  const behavioural = readFileSync(join(SRC_ROOT, 'utils', 'windowsConsoleWindow.test.ts'), 'utf8');
  assert.match(behavioural, /CONTROL: spawning without windowsHide shows a console window/);
  assert.match(behavioural, /resolves\.toBe\('visible'\)/);
  assert.match(behavioural, /runProbe/);
});
