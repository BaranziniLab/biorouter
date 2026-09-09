/**
 * Isolated config roots for the Electron end-to-end suite.
 *
 * ⚠ **INVARIANT 1 — SANDBOX EVERY LAUNCH.** A dev launch of the desktop app
 * *writes* the config tree it is pointed at: it strips extensions it does not
 * recognise, resets the provider, and has wiped a developer's real
 * `~/.config/biorouter` before. `BIOROUTER_PATH_ROOT` is what redirects it
 * (`src/main.ts` points the config dir at `<root>/config`, and the daemon puts
 * its sessions/schedule under `<root>/data`), so **no spec in this directory may
 * call `electron.launch` without one** — not even a spec that only reads. The
 * companion half of the invariant lives in `app.ts`: a `--user-data-dir` under
 * the same root, because `BIOROUTER_PATH_ROOT` does nothing for the *renderer's*
 * state — localStorage, the sidebar disclosure, the seen-announcements list —
 * which otherwise lands in the shared default Electron profile.
 *
 * Sourcing: a seeded root (a configured provider + secrets, so the app boots
 * straight into chat) is copied per run from `BIOROUTER_E2E_SEED`, defaulting to
 * `~/biorouter-runs/seed-config`. With no seed on disk a bare skeleton is built
 * instead — an empty sandbox is a poor fixture but it is still a sandbox, and
 * failing *open* onto the real config is the one outcome that is never allowed.
 */

import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';

/** A config/data root the app may be pointed at, plus how to dispose of it. */
export interface Sandbox {
  /** Value for `BIOROUTER_PATH_ROOT`. Holds `config/`, `data/` and `electron/`. */
  readonly root: string;
  /** True when this process created the directory and may delete it. */
  readonly owned: boolean;
  /** Removes the root, but only when this process created it. Never throws. */
  cleanup(): void;
}

/** Where a seeded root is copied from. */
export function seedRoot(): string {
  return process.env.BIOROUTER_E2E_SEED ?? path.join(os.homedir(), 'biorouter-runs', 'seed-config');
}

/**
 * The config directory the app under test actually writes to.
 *
 * For specs that assert against files on disk (an installed extension's tree, a
 * rewritten `config.yaml`). Reading `BIOROUTER_PATH_ROOT` here is what keeps the
 * assertion pointed at the same tree the launch was redirected to — hard-coding
 * `~/.config/biorouter` next to a sandboxed launch produces a test that can only
 * fail, and hard-coding it next to an *un*-sandboxed launch produces one that
 * mutates the developer's real config.
 */
export function configRoot(): string {
  // BIOROUTER_PATH_ROOT first: a spec that spawns the app itself sets it, and
  // that is the root the app is actually using. BIOROUTER_E2E_PATH_ROOT second,
  // so a root the operator exported is shared by specs that hand off to each
  // other (bioroffice-install installs, bioroffice-verify reads it back).
  for (const value of [process.env.BIOROUTER_PATH_ROOT, process.env.BIOROUTER_E2E_PATH_ROOT]) {
    if (value && value.trim() !== '') return path.join(value, 'config');
  }
  return path.join(os.homedir(), '.config', 'biorouter');
}

/**
 * The root to run against.
 *
 * An externally supplied `BIOROUTER_E2E_PATH_ROOT` wins and is left alone —
 * the operator who exported it owns its lifetime, so `cleanup()` is a no-op.
 * Otherwise a fresh `mkdtemp` copy of the seed is made and deleted afterwards.
 */
export function createSandbox(): Sandbox {
  const provided = process.env.BIOROUTER_E2E_PATH_ROOT;
  if (provided && provided.trim() !== '') {
    ensureSkeleton(provided);
    return { root: provided, owned: false, cleanup: () => {} };
  }

  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'biorouter-e2e-'));
  const seed = seedRoot();
  if (fs.existsSync(seed)) {
    fs.cpSync(seed, root, { recursive: true });
  }
  ensureSkeleton(root);
  return {
    root,
    owned: true,
    cleanup: () => {
      try {
        fs.rmSync(root, { recursive: true, force: true });
      } catch {
        // A leaked temp dir is noise; a teardown that throws fails a green run.
      }
    },
  };
}

/**
 * Makes a root usable whether it came from the seed or from nothing.
 *
 * The schedule is always rewritten to `[]`: the seed carries a `daily-meditation`
 * job whose `source` is a YAML file *outside* the sandbox, so leaving it in place
 * lets a scheduled run reach out of the box the sandbox exists to draw.
 */
function ensureSkeleton(root: string): void {
  for (const child of ['config', 'data', 'electron']) {
    fs.mkdirSync(path.join(root, child), { recursive: true });
  }
  fs.writeFileSync(path.join(root, 'data', 'schedule.json'), '[]');
}
