/**
 * appFolder.ts — Biorouter's own folder on Windows, and the one-time move out
 * of the folder earlier releases used (R-31).
 *
 * The desktop main process's half of `crates/biorouter-mcp/src/app_folder/`,
 * whose module header states the whole rule. In short, for each base
 * (`%APPDATA%` for config, data and logs, `%LOCALAPPDATA%` for the cache):
 *
 * 1. Move, once per process, before first use: when the legacy folder exists
 *    and the current one does not, drop the empty marker into the legacy
 *    folder, create `<base>\UCSF`, rename the legacy folder to the current one,
 *    and remove the legacy author folder if that left it empty.
 * 2. Resolve on every call: current if it exists, else legacy if it still
 *    exists (the move could not happen this run), else current.
 *
 * The daemon does both of those and a third step this process cannot: it
 * rewrites the absolute paths stored in the moved folder (the session database
 * among them), prompted by the marker. The main process runs the move before it
 * starts the daemon, so both always see the same folder; whichever of them
 * renames first wins, and the other finds the current folder.
 *
 * Both implementations run one table of cases,
 * `crates/biorouter-mcp/src/app_folder/resolution-cases.json`, and a Rust test
 * reads the four constants below from this file and compares them with its own.
 *
 * macOS and Linux use the XDG layout, which has no author segment, so nothing
 * here runs there.
 */

import * as fs from 'fs';
import * as path from 'path';

/** `%APPDATA%\UCSF\biorouter`. Not `Biorouter`: that is Electron's own folder
 *  on Windows (the desktop `productName`), and the two must stay apart. */
export const APP_AUTHOR = 'UCSF';
export const APP_NAME = 'biorouter';
/**
 * The author folder earlier releases used on Windows. The move has to name the
 * folder to find it, so this is the one place the desktop code writes the name;
 * the daemon's copy is `LEGACY_APP_AUTHOR` in `app_folder/mod.rs`, pinned equal
 * to this one by a Rust test.
 */
export const LEGACY_APP_AUTHOR = 'Block';
/** An empty file dropped into the legacy folder just before it is renamed. In
 *  the current folder it tells the daemon the stored paths still need
 *  rewriting. */
export const MOVE_MARKER = '.folder-move-pending';

type Join = (...parts: string[]) => string;

/** One base directory's two candidate folders, and their author folders. */
export interface AppRoot {
  current: string;
  legacy: string;
  currentParent: string;
  legacyParent: string;
}

export function appRootUnder(base: string, join: Join = path.join): AppRoot {
  return {
    current: join(base, APP_AUTHOR, APP_NAME),
    legacy: join(base, LEGACY_APP_AUTHOR, APP_NAME),
    currentParent: join(base, APP_AUTHOR),
    legacyParent: join(base, LEGACY_APP_AUTHOR),
  };
}

/** The filesystem calls the move makes; tests swap in a rename that refuses. */
export interface FolderIo {
  exists(target: string): boolean;
  createEmptyFile(target: string): void;
  mkdir(target: string): void;
  rename(from: string, to: string): void;
  /** Removes an EMPTY directory only, and throws otherwise. */
  rmdir(target: string): void;
}

export const NODE_FOLDER_IO: FolderIo = {
  exists: (target) => fs.existsSync(target),
  createEmptyFile: (target) => fs.writeFileSync(target, ''),
  mkdir: (target) => {
    fs.mkdirSync(target, { recursive: true });
  },
  rename: (from, to) => fs.renameSync(from, to),
  rmdir: (target) => fs.rmdirSync(target),
};

/** Rule 2: the folder to read and write right now. */
export function resolveAppRoot(root: AppRoot, io: Pick<FolderIo, 'exists'> = NODE_FOLDER_IO) {
  if (io.exists(root.current)) return root.current;
  if (io.exists(root.legacy)) return root.legacy;
  return root.current;
}

export type MoveToken = 'fresh' | 'already-moved' | 'moved' | 'both-exist' | 'move-failed';

export interface MoveOutcome {
  token: MoveToken;
  error?: unknown;
}

/**
 * `[legacy exists, current exists]` as of one instant. Two separate checks can
 * straddle the daemon's rename and see the legacy folder before it and the
 * current one after it; reading `legacy` on both sides of `current` and
 * retrying when they disagree gives a consistent pair.
 */
function snapshot(root: AppRoot, io: FolderIo): [boolean, boolean] {
  for (;;) {
    const legacy = io.exists(root.legacy);
    const current = io.exists(root.current);
    if (io.exists(root.legacy) === legacy) return [legacy, current];
  }
}

/** After a failed step: did another process finish the move, or create the
 *  current folder, in the meantime? */
function settledByAnotherProcess(root: AppRoot, io: FolderIo): MoveOutcome | undefined {
  const [legacy, current] = snapshot(root, io);
  if (!legacy && current) return { token: 'already-moved' };
  if (legacy && current) return { token: 'both-exist' };
  return undefined;
}

function attemptMove(root: AppRoot, writeMarker: boolean, io: FolderIo): MoveOutcome {
  try {
    // Never removed again on failure: the daemon may be about to rename the
    // tree with it inside, and an empty file in a folder that stays costs
    // nothing.
    if (writeMarker) io.createEmptyFile(path.join(root.legacy, MOVE_MARKER));
  } catch (error) {
    return settledByAnotherProcess(root, io) ?? { token: 'move-failed', error };
  }
  const createdParent = !io.exists(root.currentParent);
  try {
    if (createdParent) io.mkdir(root.currentParent);
    io.rename(root.legacy, root.current);
    return { token: 'moved' };
  } catch (error) {
    const settled = settledByAnotherProcess(root, io);
    if (settled) return settled;
    // One attempt only: this runs on the main thread before the first window,
    // and the daemon, which starts next, retries with a short backoff.
    if (createdParent) {
      try {
        io.rmdir(root.currentParent);
      } catch {
        /* not empty, or already gone */
      }
    }
    return { token: 'move-failed', error };
  }
}

/** Rule 1 for one base directory. `writeMarker` is true for the roaming
 *  folder, the only one whose stores hold paths that need rewriting. */
export function moveLegacyRoot(
  root: AppRoot,
  writeMarker: boolean,
  io: FolderIo = NODE_FOLDER_IO
): MoveOutcome {
  const [legacy, current] = snapshot(root, io);
  let outcome: MoveOutcome;
  if (!legacy && !current) outcome = { token: 'fresh' };
  else if (!legacy) outcome = { token: 'already-moved' };
  else if (current) outcome = { token: 'both-exist' };
  else outcome = attemptMove(root, writeMarker, io);

  if (!io.exists(root.legacy)) {
    // The author folder may hold other apps' folders; `rmdir` only removes an
    // empty one. Also tidies a folder an interrupted earlier run left empty.
    try {
      io.rmdir(root.legacyParent);
    } catch {
      /* not empty, or not there */
    }
  }
  return outcome;
}

type Env = Record<string, string | undefined>;

/**
 * The two Windows base directories: `APPDATA` and `LOCALAPPDATA` when set and
 * non-empty, else `<home>\AppData\Roaming` and `<home>\AppData\Local`.
 * etcetera has a middle rung (the known-folder API) this cannot reproduce; on
 * a normal profile both name the same directory, and every Electron process we
 * ship has both variables set.
 */
export function windowsBases(env: Env, homedir: string, join: Join = path.join) {
  const pick = (value: string | undefined, ...fallback: string[]) =>
    value && value.length > 0 ? value : join(homedir, ...fallback);
  return {
    roaming: pick(env.APPDATA, 'AppData', 'Roaming'),
    local: pick(env.LOCALAPPDATA, 'AppData', 'Local'),
  };
}

export interface FolderMove {
  /** Why nothing was looked at, when nothing was. */
  skipped?: 'not-windows' | 'sandboxed';
  roaming?: { root: AppRoot; outcome: MoveOutcome };
  local?: { root: AppRoot; outcome: MoveOutcome };
}

export interface ProcessInputs {
  platform: typeof process.platform;
  env: Env;
  homedir: string;
  io?: FolderIo;
  join?: Join;
}

/** Rule 1 for a whole process, from explicit inputs so the Windows branch is
 *  tested on every OS. Never throws. */
export function moveForProcess(inputs: ProcessInputs): FolderMove {
  if (inputs.platform !== 'win32') return { skipped: 'not-windows' };
  // The sandbox owns every path; same blank-is-absent rule as the resolvers.
  const sandboxRoot = inputs.env.BIOROUTER_PATH_ROOT;
  if (sandboxRoot && sandboxRoot.trim().length > 0) return { skipped: 'sandboxed' };
  const join = inputs.join ?? path.join;
  const io = inputs.io ?? NODE_FOLDER_IO;
  const bases = windowsBases(inputs.env, inputs.homedir, join);
  const roaming = appRootUnder(bases.roaming, join);
  const local = appRootUnder(bases.local, join);
  return {
    roaming: { root: roaming, outcome: moveLegacyRoot(roaming, true, io) },
    local: { root: local, outcome: moveLegacyRoot(local, false, io) },
  };
}

/** One line per thing worth saying about a process's move. */
export function describeFolderMove(move: FolderMove): { level: 'info' | 'warn'; text: string }[] {
  const lines: { level: 'info' | 'warn'; text: string }[] = [];
  for (const report of [move.roaming, move.local]) {
    if (!report) continue;
    const { current, legacy } = report.root;
    switch (report.outcome.token) {
      case 'moved':
        lines.push({
          level: 'info',
          text: `moved Biorouter's folder from ${legacy}, the folder earlier releases used, to ${current}`,
        });
        break;
      case 'both-exist':
        lines.push({
          level: 'warn',
          text: `both ${current} and ${legacy} (the folder earlier releases used) exist; using ${current} and leaving the other untouched`,
        });
        break;
      case 'move-failed':
        lines.push({
          level: 'warn',
          text: `could not move ${legacy} (the folder earlier releases used) to ${current}: ${String(report.outcome.error)}. Using it as it is; the daemon tries again as it starts`,
        });
        break;
      default:
        break;
    }
  }
  return lines;
}
