/**
 * The Electron main process runs window compositing, IPC and input handling on
 * one thread. A synchronous child process there freezes the entire app for its
 * full duration — which is what issue #88 was: `spawnSync('biorouter doctor')`
 * with a 15 s budget, fired 4 s after launch, showing the macOS wait cursor and
 * an unresponsive window.
 *
 * The fix is structural (every probe is now `execFile`-based), so the guard has
 * to be structural too: the modules on the startup path may not reach for a
 * synchronous spawn again. A behavioural test cannot catch this — a reintroduced
 * `spawnSync` still returns the right answer, it just blocks while doing it.
 */
import { describe, it, expect } from 'vitest';
import { readFileSync, readdirSync } from 'fs';
import * as path from 'path';
import {
  STARTUP_SCHEDULE,
  MIN_STARTUP_TASK_GAP_MS,
  STARTUP_UPDATER_SETUP_DELAY_MS,
  STARTUP_DEPENDENCY_CHECK_DELAY_MS,
  STARTUP_EXTENSION_CHECK_DELAY_MS,
} from './startupSchedule';

const SRC = path.join(__dirname, '..');

// Everything reachable from app-ready, or from a timer armed during app-ready.
const STARTUP_PATH_MODULES = [
  'main.ts',
  'biorouterd.ts',
  'utils/dependencyChecker.ts',
  'utils/extensionUpdater.ts',
  'utils/autoUpdater.ts',
  'utils/githubUpdater.ts',
  'utils/updateCheckSchedule.ts',
  'utils/macUpdateInstall.ts',
  'utils/resumableDownload.ts',
  'utils/resumableUpdaterDownloads.ts',
  'utils/electronNetTransport.ts',
  // ⚠ Added after the fact: `ensureWinShims()` is awaited from `appMain()`
  // BEFORE the first window exists, and it held an `fs.cpSync` of the ~120 MB
  // bundled MinGit tree. It was on the startup path from the day it was
  // written and this list never named it.
  'utils/winShims.ts',
];

// `spawnSync` et al. block the event loop until the child exits.
const BLOCKING_SPAWN = /\b(spawnSync|execSync|execFileSync)\s*\(/;

// ⚠ **Bulk synchronous FILESYSTEM calls, which #88's guard did not cover.**
//
// The original defect was a synchronous spawn, so this file banned spawns. But
// `fs.cpSync` of the bundled MinGit tree - thousands of files, ~120 MB, every
// write scanned by Defender on a Windows first run - parked the main thread
// before the first window existed, and passed every assertion here.
//
// Only the APIs that are recursive by intent are listed. `existsSync` and
// `statSync` are single stats, they appear throughout legitimately, and banning
// them would make this guard noise that people learn to widen.
const BLOCKING_BULK_FS = /\b(cpSync|rmSync|rmdirSync)\s*\(/;

/**
 * The bulk-filesystem check runs over every startup module EXCEPT `main.ts`.
 *
 * ⚠ **This is a real gap, stated rather than hidden.** `main.ts` is not a
 * startup module - it is the whole main process, including every
 * `ipcMain.handle` in the app. `brxt:uninstall` legitimately calls
 * `fsSync.rmSync` on a directory the user asked to delete, long after the
 * window exists. Scanning the file wholesale reports that as a startup
 * blocker, and a guard that cries wolf on correct code is one somebody widens
 * or deletes.
 *
 * The cost: a bulk synchronous copy added directly to `appMain()` would not be
 * caught here. Nothing in the file's structure separates its startup path from
 * its handlers - `appMain()` sits BELOW the IPC registrations, so position
 * cannot be used either. The modules below are focused enough that a recursive
 * synchronous call anywhere in them is suspicious, which is what makes the
 * check meaningful for them and not for `main.ts`.
 */
const BULK_FS_MODULES = STARTUP_PATH_MODULES.filter((m) => m !== 'main.ts');

function sourceOf(rel: string): string {
  return readFileSync(path.join(SRC, rel), 'utf8');
}

/** Strip block and line comments so a mention in prose isn't read as a call. */
function stripComments(src: string): string {
  return src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
}

describe('startup path never blocks the main thread', () => {
  for (const rel of STARTUP_PATH_MODULES) {
    it(`${rel} contains no synchronous child-process call`, () => {
      const code = stripComments(sourceOf(rel));
      const offenders = code
        .split('\n')
        .map((line, i) => ({ line: line.trim(), n: i + 1 }))
        .filter(({ line }) => BLOCKING_SPAWN.test(line));

      expect(
        offenders,
        `${rel} reintroduces a blocking spawn on the Electron main thread (#88). ` +
          `Use runProbe() from utils/dependencyChecker, or child_process.spawn.\n` +
          offenders.map((o) => `  line ${o.n}: ${o.line}`).join('\n')
      ).toEqual([]);
    });
  }

  for (const rel of BULK_FS_MODULES) {
    it(`${rel} contains no bulk synchronous filesystem call`, () => {
      const code = stripComments(sourceOf(rel));
      const offenders = code
        .split('\n')
        .map((line, i) => ({ line: line.trim(), n: i + 1 }))
        .filter(({ line }) => BLOCKING_BULK_FS.test(line));

      expect(
        offenders,
        `${rel} blocks the Electron main thread with a recursive filesystem call (#88). ` +
          `A 120 MB copy freezes the app exactly as a synchronous spawn does; use the ` +
          `fs.promises equivalent.\n` +
          offenders.map((o) => `  line ${o.n}: ${o.line}`).join('\n')
      ).toEqual([]);
    });
  }

  it('does not import a synchronous spawn helper', () => {
    for (const rel of STARTUP_PATH_MODULES) {
      const code = stripComments(sourceOf(rel));
      const imports = code.match(/import\s+\{[^}]*\}\s+from\s+'child_process'/g) ?? [];
      for (const imp of imports) {
        expect(imp, `${rel} imports a blocking spawn (#88)`).not.toMatch(
          /spawnSync|execSync|execFileSync/
        );
      }
    }
  });
});

describe('startup schedule', () => {
  it('runs its tasks in a fixed, non-overlapping order', () => {
    const delays = STARTUP_SCHEDULE.map((t) => t.delayMs);
    expect(delays).toEqual([...delays].sort((a, b) => a - b));

    for (let i = 1; i < STARTUP_SCHEDULE.length; i++) {
      const gap = STARTUP_SCHEDULE[i].delayMs - STARTUP_SCHEDULE[i - 1].delayMs;
      expect(
        gap,
        `${STARTUP_SCHEDULE[i - 1].name} → ${STARTUP_SCHEDULE[i].name} is only ${gap}ms apart`
      ).toBeGreaterThanOrEqual(MIN_STARTUP_TASK_GAP_MS);
    }
  });

  it('keeps every task out of the renderer first-paint window', () => {
    for (const task of STARTUP_SCHEDULE) {
      expect(task.delayMs, `${task.name} fires too early`).toBeGreaterThanOrEqual(2_000);
    }
  });

  it('orders updater setup before the dependency check before the extension check', () => {
    expect(STARTUP_UPDATER_SETUP_DELAY_MS).toBeLessThan(STARTUP_DEPENDENCY_CHECK_DELAY_MS);
    expect(STARTUP_DEPENDENCY_CHECK_DELAY_MS).toBeLessThan(STARTUP_EXTENSION_CHECK_DELAY_MS);
  });

  it('main.ts wires the schedule constants rather than inline numbers', () => {
    const main = sourceOf('main.ts');
    expect(main).toContain('STARTUP_UPDATER_SETUP_DELAY_MS');
    expect(main).toContain('setupDependencyChecker(STARTUP_DEPENDENCY_CHECK_DELAY_MS)');
    expect(main).toContain('scheduleExtensionUpdateCheck(STARTUP_EXTENSION_CHECK_DELAY_MS)');
  });
});

describe('runProbe shell selection', () => {
  // A Windows-only regression that no test on this machine would otherwise
  // exercise: routing an ABSOLUTE path through cmd.exe re-parses it as a command
  // line, so a bundled CLI under `C:\Program Files\…` splits at the space and the
  // probe fails on exactly the machines it ships to. A BARE name must still go
  // through the shell, or `npm`/`npx` (`.cmd` wrappers, not `.exe`s) are unfindable.
  const source = sourceOf('utils/dependencyChecker.ts');

  it('decides from the command rather than passing a constant', () => {
    expect(source).toContain('shell: needsShell(cmd)');
    expect(source).not.toMatch(/shell:\s*process\.platform === 'win32'/);
  });

  it('treats a path as a path and a bare name as a bare name', () => {
    const isPath = /[\\/]/;
    expect(isPath.test('C:\\Program Files\\Biorouter\\bin\\biorouter.exe')).toBe(true);
    expect(isPath.test('/Applications/Biorouter.app/Contents/Resources/bin/biorouter')).toBe(true);
    expect(isPath.test('npm')).toBe(false);
    expect(isPath.test('uv')).toBe(false);
  });
});

describe('test files that reach Electron must mock it', () => {
  /**
   * CI installs npm dependencies without the Electron binary, so evaluating the
   * real `electron` module throws at IMPORT time. A suite that fails to load
   * reports as a failed *suite* while every test inside it is silently skipped —
   * the run said "2761 passed, 0 failed" with two suites never executed. And a
   * local run cannot catch it, because a dev machine has the binary.
   *
   * So the rule is checked at the source: a test importing one of these modules
   * pulls in `electron` transitively and has to mock it.
   */
  const REACHES_ELECTRON = ['./dependencyChecker', './logger', './mainThreadWatchdog'];

  const testFiles = readdirSync(path.join(SRC, 'utils')).filter((f) => /\.test\.tsx?$/.test(f));

  it('finds the test files to check', () => {
    expect(testFiles.length).toBeGreaterThan(0);
  });

  for (const file of testFiles) {
    it(`${file} mocks electron if it imports a module that loads it`, () => {
      const code = readFileSync(path.join(SRC, 'utils', file), 'utf8');
      const importsElectronReacher = REACHES_ELECTRON.some((m) =>
        // A `import type {...}` is erased at compile time and loads nothing.
        new RegExp(`import\\s+(?!type\\s)[^;]*from\\s+'${m.replace('.', '\\.')}'`).test(code)
      );
      if (!importsElectronReacher) return;

      expect(
        code,
        `${file} imports a module that loads 'electron'. Without vi.mock('electron', …) ` +
          `the whole suite fails to LOAD in CI and its tests are skipped silently.`
      ).toMatch(/vi\.mock\(\s*'electron'/);
    });
  }
});
