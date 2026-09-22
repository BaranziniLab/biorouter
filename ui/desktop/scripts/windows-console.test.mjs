/**
 * Does BioRouter's desktop app put black console windows on a Windows screen?
 *
 * # Why this file exists, and why reading source could not answer it
 *
 * On Windows a console-subsystem child of a process that owns no console gets a
 * NEW, VISIBLE console — that is issue #368, "频繁跳黑框". The obvious lever is
 * Node's `windowsHide`, whose documented default is `false`. Reasoning stops
 * there and gets the answer wrong, twice over:
 *
 *  1. **Electron already forces the hiding branch.** It sets
 *     `EnvironmentFlags::kHideConsoleWindows` on every Node environment it
 *     creates (shell/common/node_bindings.cc, unconditionally, since Electron
 *     16), so libuv enters its hide block for every spawn whether or not the
 *     caller passed `windowsHide`. Source-reading the app's own spawn options
 *     therefore predicts nothing about the app's behaviour.
 *  2. **What actually decides it is the stdio shape.** libuv only ORs
 *     `CREATE_NO_WINDOW` in if NO stdio entry is an inherited fd
 *     (src/win/process.c, the loop at ~1034-1042, deliberate since libuv
 *     491848a0ad20 in 2017 — inheriting a console and then severing it made
 *     child output vanish). One `stdio: 'inherit'`, one raw fd, or a `fork()`
 *     without `silent: true`, and the flag is never set — with `windowsHide:
 *     true` sitting right there in the diff looking like it does something.
 *
 * So the app is protected today by an EMBEDDER DETAIL it never asked for, and
 * the one thing that would break that protection is invisible in a code review.
 * Both of those are measured here rather than believed.
 *
 * # What each assertion is for
 *
 *  * The Node-level pair is the mechanism, with its control: plain Node with no
 *    `windowsHide` must show a VISIBLE console. If that control ever reports
 *    hidden, this machine cannot show the window at all and every other
 *    assertion in the file is vacuous — so it is an assertion, never a skip.
 *  * The grandchild case settles a site that looks unguarded and is not: the
 *    Biorouter Copilot helper runs `powershell.exe` per desktop action with no
 *    creation flags, but the daemon starts that helper with CREATE_NO_WINDOW and
 *    a child inherits its parent's windowless console.
 *  * The Electron block pins the embedder behaviour. If a future Electron stops
 *    setting kHideConsoleWindows, `pipe-without-windowsHide` flips to visible
 *    and this goes red — which is the early warning the app currently does not
 *    have. And `inherit-with-windowsHide` must be VISIBLE: that is the hazard
 *    the stdio rule in console-window-census.mjs exists to prevent, and a rule
 *    guarding a hazard nobody has observed is a rule that gets deleted.
 */
import { strict as assert } from 'node:assert';
import { spawn } from 'node:child_process';
import { readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { askFromHere, probeArgs, readVerdict, scratchDir } from './windows-console-probe.mjs';

const HERE = fileURLToPath(new URL('.', import.meta.url));
const WINDOWS = process.platform === 'win32';

// The Windows CI job sets this. Without it a run on any other platform skips
// every test and exits 0, which is indistinguishable from a run that passed.
if (!WINDOWS && process.env.BIOROUTER_REQUIRE_WINDOWS_CONSOLE === '1') {
  throw new Error(
    `BIOROUTER_REQUIRE_WINDOWS_CONSOLE=1 but platform is ${process.platform}: these tests measure Windows and cannot run here`
  );
}

const onWindows = { skip: WINDOWS ? false : 'measures Windows console windows' };

/**
 * No black box on screen. Measured on real Windows: a CREATE_NO_WINDOW child
 * reports `none`, not `hidden` — Microsoft documents the flag as leaving "the
 * console handle for the application ... not set", so there is no window to
 * hide. `hidden` is accepted too because it is the weaker of the two outcomes
 * and asserting only `none` would fail on any host that does give the child a
 * console; both are pass conditions and `visible` is the only failure.
 */
function assertNoWindow(verdict, message) {
  assert.ok(
    verdict === 'none' || verdict === 'hidden',
    `${message} (the probe reported ${JSON.stringify(verdict)})`
  );
}
const MINUTE = 60_000;

const PWSH = 'powershell.exe';

test(
  'CONTROL: a plain Node spawn with no windowsHide shows a console window',
  onWindows,
  async () => {
    const verdict = await askFromHere(PWSH, probeArgs, { stdio: 'pipe' }, 'node control');
    assert.equal(
      verdict,
      'visible',
      'the control did not show a window, so this machine cannot demonstrate the bug and every other assertion here proves nothing'
    );
  }
);

test('windowsHide: true hides it under plain Node', onWindows, async () => {
  const verdict = await askFromHere(
    PWSH,
    probeArgs,
    { stdio: 'pipe', windowsHide: true },
    'node hidden'
  );
  assertNoWindow(verdict, 'windowsHide: true left a console window on screen');
});

test('a grandchild behind a hidden cmd.exe stays hidden', onWindows, async () => {
  const verdict = await askFromHere(
    'cmd.exe',
    (out) => ['/d', '/s', '/c', PWSH, ...probeArgs(out)],
    { stdio: 'pipe', windowsHide: true },
    'hidden grandchild'
  );
  assertNoWindow(
    verdict,
    'hiding a shell must be enough for what it runs, or flagging one MCP server would not quiet the tools it launches'
  );
});

test(
  'CONTROL: the same grandchild behind an unhidden cmd.exe shows a window',
  onWindows,
  async () => {
    const verdict = await askFromHere(
      'cmd.exe',
      (out) => ['/d', '/s', '/c', PWSH, ...probeArgs(out)],
      { stdio: 'pipe' },
      'visible grandchild'
    );
    assert.equal(verdict, 'visible');
  }
);

// ── Inside the real Electron ─────────────────────────────────────────────────

function runElectronProbe() {
  const dir = scratchDir();
  const resultsPath = join(dir, 'results.json');
  return new Promise((resolve, reject) => {
    void import('electron')
      .then(({ default: electronPath }) => {
        const child = spawn(
          electronPath,
          [join(HERE, 'windows-console-electron-probe.cjs'), resultsPath],
          { stdio: 'pipe' }
        );
        let stderr = '';
        child.stderr?.on('data', (chunk) => (stderr += chunk.toString()));
        child.on('error', reject);
        child.on('close', (code) => {
          try {
            resolve(JSON.parse(readFileSync(resultsPath, 'utf8')));
          } catch (error) {
            reject(
              new Error(
                `Electron probe wrote no results (exit ${code}). This is a failure, not an excuse to skip: the job installs Electron on purpose.\nstderr:\n${stderr}\n${error.message}`
              )
            );
          } finally {
            rmSync(dir, { recursive: true, force: true });
          }
        });
      })
      .catch(reject);
  });
}

let electronResults = null;
async function electron() {
  if (!electronResults) electronResults = await runElectronProbe();
  assert.equal(electronResults.fatal, undefined, `probe crashed: ${electronResults.fatal}`);
  return electronResults;
}

test(
  'Electron hides a piped console child even with no windowsHide (kHideConsoleWindows)',
  { ...onWindows, timeout: 3 * MINUTE },
  async () => {
    const results = await electron();
    assertNoWindow(
      results['pipe-without-windowsHide'],
      'Electron has stopped forcing kHideConsoleWindows. Every spawn in the main process that does not pass windowsHide: true is now a black box on screen'
    );
  }
);

test(
  'Electron hides it with windowsHide: true as well',
  { ...onWindows, timeout: 3 * MINUTE },
  async () => {
    const results = await electron();
    assertNoWindow(results['pipe-with-windowsHide'], 'windowsHide: true showed a window');
    assertNoWindow(results['ignore-without-windowsHide'], "stdio: 'ignore' showed a window");
  }
);

test(
  'THE HAZARD: inherited stdio shows a window even with windowsHide: true',
  { ...onWindows, timeout: 3 * MINUTE },
  async () => {
    const results = await electron();
    assert.equal(
      results['inherit-with-windowsHide'],
      'visible',
      'an inherited-fd spawn no longer shows a console window. If libuv has changed, the stdio rule in console-window-census.mjs is guarding nothing and should be re-derived rather than kept out of habit.'
    );
  }
);

// ── The probe itself ─────────────────────────────────────────────────────────

test('the probe reports nothing rather than passing when it fails to run', onWindows, () => {
  const dir = scratchDir();
  try {
    assert.throws(() => readVerdict(join(dir, 'absent.txt'), 'self-test'), /NOTHING was measured/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
