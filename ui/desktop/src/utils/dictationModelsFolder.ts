/**
 * dictationModelsFolder.ts: the speech model folder that Settings' **Show in Finder** (or Show in
 * Explorer) may create and open, as the main process derives it (DD-19, DD-66).
 *
 * The daemon reports its folder as `status.models_dir`, and that string reaches the main process
 * through the renderer (`dictation:show-models-folder`). So main never creates or opens a folder a
 * renderer names. It derives this account's folder itself, by the rule the daemon's
 * `Paths::in_local_data_dir("dictation/models")` follows, and reads the renderer's string only as
 * a hint that must name that same folder. Whatever a renderer sends, the one folder this can
 * create or open is this account's own speech model folder.
 *
 * The rule (`crates/biorouter/src/config/paths.rs`, `crates/biorouter-mcp/src/paths.rs`):
 *
 * 1. `BIOROUTER_PATH_ROOT`, when it is not blank: `<root>/data/dictation/models`. A relative root
 *    names a different folder in every process (see `./biorouterPaths.ts`), so it admits none.
 * 2. Windows: `<local root>\data\dictation\models`, where the local root is
 *    `%LOCALAPPDATA%\UCSF\biorouter`, or the legacy folder while the daemon has not moved it yet
 *    (`./appFolder.ts`). It is not the roaming `%APPDATA%` folder: a model is 0.3 to 0.6 GB.
 * 3. Elsewhere, macOS included (etcetera's XDG strategy): `$XDG_DATA_HOME/biorouter/dictation/models`,
 *    or `~/.local/share/biorouter/dictation/models` when that variable is unset or not absolute.
 *
 * The desktop's own daemon is started with this process's environment (`biorouterd.ts`; a
 * development profile sets `BIOROUTER_PATH_ROOT` for both), so the two agree. When they do not
 * (a shared daemon another program started with another environment), the hint names another
 * folder and nothing is created or opened.
 */

import * as path from 'path';
import { APP_NAME, appRootUnder, resolveAppRoot, windowsBases, type Presence } from './appFolder';

/** What the derivation reads; `process` and `os` in the app. */
export interface DictationFolderInputs {
  platform: typeof process.platform;
  env: Record<string, string | undefined>;
  homedir: string;
  /** `path.win32` lets the suite run the Windows branch on every OS. */
  pathImpl?: typeof path.posix | typeof path.win32;
  presence?: (target: string) => Presence;
}

/** The longest hint read; a real folder is far shorter. */
export const MAX_FOLDER_HINT = 4096;

/** This account's speech model folder, or `null` when it cannot be named as an absolute path. */
export function dictationModelsFolderFor(inputs: DictationFolderInputs): string | null {
  const p = inputs.pathImpl ?? path;
  const root = inputs.env.BIOROUTER_PATH_ROOT;
  let localData: string;
  if (root && root.trim().length > 0) {
    // As given, untrimmed, as `Paths` reads it: only a blank root reads as absent.
    localData = p.join(root, 'data');
  } else if (inputs.platform === 'win32') {
    const { local } = windowsBases(inputs.env, inputs.homedir, p.join);
    localData = p.join(resolveAppRoot(appRootUnder(local, p.join), inputs.presence), 'data');
  } else {
    const xdg = inputs.env.XDG_DATA_HOME;
    localData =
      xdg && p.isAbsolute(xdg)
        ? p.join(xdg, APP_NAME)
        : p.join(inputs.homedir, '.local', 'share', APP_NAME);
  }
  const folder = p.join(localData, 'dictation', 'models');
  return p.isAbsolute(folder) ? folder : null;
}

/**
 * The folder to open for `hint`: this account's speech model folder when the hint names it, else
 * `null`. The daemon builds its path with `PathBuf::join`, which keeps a separator as written
 * (`…\data\dictation/models` on Windows), so the hint is compared after normalizing its
 * separators, and case blind on Windows; a hint with a `..` segment is refused rather than
 * resolved. What is returned is always the derived folder, never the hint.
 */
export function admitDictationModelsFolder(
  hint: unknown,
  inputs: DictationFolderInputs
): string | null {
  if (typeof hint !== 'string' || hint.length === 0 || hint.length > MAX_FOLDER_HINT) return null;
  const p = inputs.pathImpl ?? path;
  if (!p.isAbsolute(hint) || hint.split(/[\\/]/).includes('..')) return null;
  const folder = dictationModelsFolderFor(inputs);
  if (!folder) return null;
  const named = p.normalize(hint);
  const same =
    inputs.platform === 'win32' ? named.toLowerCase() === folder.toLowerCase() : named === folder;
  return same ? folder : null;
}

/** The file system and shell calls {@link showDictationModelsFolder} makes; fakes in the suite. */
export interface ShowFolderDeps {
  mkdir(target: string): Promise<unknown>;
  realpath(target: string): Promise<string>;
  isPlainDirectory(target: string): Promise<boolean>;
  /** `shell.openPath`: `''` when it opened, else the error. */
  openPath(target: string): Promise<string>;
}

/**
 * Opens this account's speech model folder in the file manager when `hint` names it, and answers
 * whether it opened. The folder exists only after a first install, and the air gapped path puts
 * files there before one (DD-24), so it is created when missing; nothing is created when the hint
 * names another folder. It opens only when its real path is a plain folder, never a bundle
 * (`.app` and its kind), which `open` would launch.
 */
export async function showDictationModelsFolder(
  hint: unknown,
  inputs: DictationFolderInputs,
  deps: ShowFolderDeps
): Promise<boolean> {
  const folder = admitDictationModelsFolder(hint, inputs);
  if (!folder) return false;
  await deps.mkdir(folder);
  const real = await deps.realpath(folder);
  if (!(await deps.isPlainDirectory(real))) return false;
  return (await deps.openPath(real)) === '';
}
