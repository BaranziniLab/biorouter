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
 *  2. **There are TWO levers, not one, and they cover different sites.**
 *     `CREATE_NO_WINDOW` gives the child no console at all, but libuv only ORs
 *     it in when NO stdio entry is an inherited fd (src/win/process.c, the loop
 *     at ~1034-1042, deliberate since libuv 491848a0ad20 in 2017 — inheriting a
 *     console and then severing it made child output vanish). `SW_HIDE` is the
 *     other: libuv sets STARTF_USESHOWWINDOW unconditionally, so `windowsHide:
 *     true` also hides a console that DID get created. An inherit-stdio spawn
 *     therefore falls through the first lever and is caught only by the second
 *     — which is why every site stating `windowsHide` is the load-bearing
 *     requirement, and why a site that both inherits an fd and omits the option
 *     is the one shape that puts a black box on screen.
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
 *  * The `cmd.exe` grandchild case measures only that specific shell path. The
 *    Biorouter Copilot Go helper starts PowerShell separately and must give
 *    that child its own creation flags.
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

/**
 * What this harness can measure, asked once.
 *
 * A console-subsystem parent that owns NO console does not hand its children a
 * fresh one — they get none. Measured twice, independently: a peer's agent shell
 * returned `none` for all six cases of an earlier version of this file, and
 * GitHub's windows-latest runner does the same, because the runner agent is a
 * service and `node` under it has no console. Both readings look exactly like
 * "Node hides console windows by default", and both are the harness, not Node.
 *
 * So the Node-level cases below are only meaningful from a console-BEARING
 * parent, and they say so rather than asserting into the void. What keeps the
 * file from passing vacuously in that case is the Electron block: Electron is a
 * GUI-subsystem parent, which DOES give a console child a new console, so its
 * controls draw a real window on the very runner where these cannot. At least
 * one control here produces `visible` in every environment, and that is the
 * property that makes the rest of the file mean anything.
 */
let harnessVerdict = null;
async function harnessConsole() {
  if (harnessVerdict === null) {
    harnessVerdict = await askFromHere(PWSH, probeArgs, { stdio: 'pipe' }, 'harness control');
  }
  return harnessVerdict;
}

test('CONTROL: does this harness own a console to hand down?', onWindows, async () => {
  const verdict = await harnessConsole();
  assert.ok(
    verdict === 'visible' || verdict === 'none',
    `a spawn with no windowsHide reported ${JSON.stringify(verdict)}. It should be 'visible' from a console-bearing parent, or 'none' from a console-less one; 'hidden' would mean something is applying SW_HIDE that nobody asked for.`
  );
  if (verdict === 'none') {
    console.log(
      '# this harness owns no console, so the Node-level cases cannot measure anything here; the Electron block carries the proof'
    );
  }
});

test('windowsHide: true hides it under plain Node', onWindows, async () => {
  if ((await harnessConsole()) !== 'visible') return;
  const verdict = await askFromHere(
    PWSH,
    probeArgs,
    { stdio: 'pipe', windowsHide: true },
    'node hidden'
  );
  assertNoWindow(verdict, 'windowsHide: true left a console window on screen');
});

test('a grandchild behind a hidden cmd.exe stays hidden', onWindows, async () => {
  if ((await harnessConsole()) !== 'visible') return;
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
    if ((await harnessConsole()) !== 'visible') return;
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
  "CONTROL: inherited stdio defeats Electron's own console hiding",
  { ...onWindows, timeout: 3 * MINUTE },
  async () => {
    // This is what makes the stdio rule in console-window-census.mjs worth
    // having. Electron forces the hiding branch for every spawn, but libuv only
    // ORs CREATE_NO_WINDOW in when NO stdio entry is an inherited fd — so an
    // inheriting site escapes the app's blanket protection. If this ever stops
    // being visible, the rule guards nothing and should be re-derived rather
    // than kept out of habit.
    const results = await electron();
    assert.equal(
      results['inherit-without-windowsHide'],
      'visible',
      'an inherited-fd spawn no longer shows a console window inside Electron'
    );
  }
);

test(
  'SETTLED: windowsHide rescues an inherit-stdio spawn, by hiding the console rather than preventing it',
  { ...onWindows, timeout: 3 * MINUTE },
  async () => {
    // Measured on real Windows, 2026-09-22, Electron 39.8.10. This case returned
    // `hidden`, and the difference between `hidden` and `none` is the whole
    // finding: `none` is what CREATE_NO_WINDOW produces, so a console WAS
    // created here — libuv really did skip the flag because an fd is inherited,
    // exactly as src/win/process.c reads. No window was drawn anyway, because
    // libuv sets STARTF_USESHOWWINDOW unconditionally and `windowsHide: true`
    // therefore also asks for SW_HIDE, which hides the console that did get
    // created.
    //
    // So the two levers are independent and this is the site where only the
    // second one is left:
    //   * CREATE_NO_WINDOW — no console at all; needs non-inherited stdio;
    //   * SW_HIDE — a console exists but is not shown; needs `windowsHide`.
    // Which makes `windowsHide: true` the load-bearing requirement, not the
    // cosmetic one it looked like when Electron appeared to be hiding
    // everything by itself.
    const results = await electron();
    assert.equal(
      results['inherit-with-windowsHide'],
      'hidden',
      `an inherit-stdio spawn with windowsHide: true now reports ${results['inherit-with-windowsHide']}. If it is 'none', libuv has started applying CREATE_NO_WINDOW despite the inherited fd. If it is 'visible', SW_HIDE has stopped reaching console children and every inherit-stdio site in the app is a black box — fix that before anything else.`
    );
  }
);

test(
  'CONTROL: inherited stdio with NO windowsHide is the black box',
  { ...onWindows, timeout: 3 * MINUTE },
  async () => {
    // The arm that makes the case above mean something. Without it, `hidden`
    // could just as well be a machine that never draws, and the requirement
    // that every site state `windowsHide` would be guarding nothing.
    const results = await electron();
    assert.equal(
      results['inherit-without-windowsHide'],
      'visible',
      `inherited stdio with no windowsHide reported ${results['inherit-without-windowsHide']} rather than showing a window. If this is no longer visible, nothing in the app can produce a console window any more and the census rules should be re-derived rather than kept out of habit.`
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
