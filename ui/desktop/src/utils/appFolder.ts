/**
 * appFolder.ts — Biorouter's own folder on Windows, as the desktop main
 * process resolves it (R-31).
 *
 * The desktop half of `crates/biorouter-mcp/src/app_folder/`, whose module
 * header states the whole rule. For each base (`%APPDATA%` for config, data
 * and logs, `%LOCALAPPDATA%` for the cache) there is a current folder,
 * `<base>\UCSF\biorouter`, and the legacy folder earlier releases used.
 *
 * This process only RESOLVES, on every call: the legacy folder while it is (or
 * may be) there and the current one is known not to be, otherwise the current
 * one. It never moves anything. The daemon moves the legacy folder as it
 * starts, when no other Biorouter process is using it, and then rewrites the
 * paths stored inside; because this process resolves per call and freezes no
 * path, a move during its run costs it nothing. (It used to move the folder
 * itself, at module load. That could rename the folder out from under a
 * command line session that had already frozen the legacy path, and a second
 * instance about to quit as a duplicate did it too.)
 *
 * When both folders exist the current one wins, and anything saved in the
 * legacy one is hidden; `legacyFolderNotice` is what the main process shows
 * the person then, naming the folder.
 *
 * Both implementations run the `resolution` cases of one table,
 * `crates/biorouter-mcp/src/app_folder/resolution-cases.json`, and a Rust test
 * reads the three constants below from this file and compares them with its
 * own.
 *
 * macOS and Linux use the XDG layout, which has no author segment, so nothing
 * here applies there.
 */

import * as fs from 'fs';
import * as path from 'path';

/** `%APPDATA%\UCSF\biorouter`. Not `Biorouter`: that is Electron's own folder
 *  on Windows (the desktop `productName`), and the two must stay apart. */
export const APP_AUTHOR = 'UCSF';
export const APP_NAME = 'biorouter';
/**
 * The author folder earlier releases used on Windows. Resolution has to name
 * the folder to find it, so this is the one place the desktop code writes the
 * name; the daemon's copy is `LEGACY_APP_AUTHOR` in `app_folder/mod.rs`,
 * pinned equal to this one by a Rust test.
 */
export const LEGACY_APP_AUTHOR = 'Block';

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

/** What a check of one folder found. `unknown`: the check failed (permission
 *  denied, an unreachable share), so it may be there. */
export type Presence = 'present' | 'absent' | 'unknown';

/** `fs.existsSync` reads every failure as "absent", which is how a check that
 *  failed for a moment could send this process into a new, empty current
 *  folder. Only "not found" and "a segment above it is a file" are absent. */
export function presenceOf(target: string): Presence {
  try {
    fs.statSync(target);
    return 'present';
  } catch (error) {
    const code = (error as { code?: string }).code;
    return code === 'ENOENT' || code === 'ENOTDIR' ? 'absent' : 'unknown';
  }
}

/**
 * `[legacy, current]` as of one instant. Two separate checks can straddle the
 * daemon's rename and see the legacy folder before it and the current one
 * after it; reading `legacy` on both sides of `current` and retrying when they
 * disagree gives a consistent pair. Bounded, as on the Rust side.
 */
function snapshot(root: AppRoot, presence: (target: string) => Presence): [Presence, Presence] {
  let pair: [Presence, Presence] = [presence(root.legacy), presence(root.current)];
  for (let attempt = 0; attempt < 16; attempt++) {
    const again = presence(root.legacy);
    if (again === pair[0]) break;
    pair = [again, presence(root.current)];
  }
  return pair;
}

/** The folder to read and write right now. */
export function resolveAppRoot(
  root: AppRoot,
  presence: (target: string) => Presence = presenceOf
): string {
  const [legacy, current] = snapshot(root, presence);
  return current === 'absent' && legacy !== 'absent' ? root.legacy : root.current;
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

export interface ProcessInputs {
  platform: typeof process.platform;
  env: Env;
  homedir: string;
  presence?: (target: string) => Presence;
  join?: Join;
}

/** Both folders exist for the roaming base, which holds config, chats and
 *  knowledge bases: the current one is in use, and the legacy one is hidden. */
export interface LegacyFolderNotice {
  current: string;
  legacy: string;
}

/**
 * Whether the person must be told about a hidden legacy folder. Only on
 * Windows, only outside a sandbox (the same blank-is-absent rule as the
 * resolvers), and only when both roaming folders are certainly there: the
 * cache's legacy folder holds nothing worth recovering.
 */
export function legacyFolderNotice(inputs: ProcessInputs): LegacyFolderNotice | undefined {
  if (inputs.platform !== 'win32') return undefined;
  const sandboxRoot = inputs.env.BIOROUTER_PATH_ROOT;
  if (sandboxRoot && sandboxRoot.trim().length > 0) return undefined;
  const join = inputs.join ?? path.join;
  const root = appRootUnder(windowsBases(inputs.env, inputs.homedir, join).roaming, join);
  const [legacy, current] = snapshot(root, inputs.presence ?? presenceOf);
  if (legacy !== 'present' || current !== 'present') return undefined;
  return { current: root.current, legacy: root.legacy };
}

/** The words of that notice, for a message box. */
export function describeLegacyFolderNotice(notice: LegacyFolderNotice) {
  return {
    title: 'Older Biorouter folder found',
    message: 'Biorouter found a second data folder, and is not using it.',
    detail:
      `Biorouter keeps its settings, chats and knowledge bases in ${notice.current}. ` +
      `Earlier releases used ${notice.legacy}, and that folder exists again, so anything ` +
      'saved there since the update (settings, chats, knowledge bases) does not appear in ' +
      'Biorouter.\n\n' +
      'This usually means an older copy of the biorouter command line tool ran after the ' +
      'update, or an earlier release of the app was opened. Update the command line tool ' +
      'when Biorouter offers to, then copy anything you need out of the older folder, and ' +
      'delete it when you no longer need it. Biorouter never moves or deletes it on its own.',
    buttons: ['Show older folder', 'OK'],
    checkboxLabel: 'Do not show this again for this folder',
  };
}
