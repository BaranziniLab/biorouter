/**
 * biorouterPaths.ts — the ONE place the main process derives Biorouter's
 * config directory.
 *
 * ⚠ **Every main-process consumer must come through here.** Issue #146 was
 * fixed once, in `extensionUpdater.ts`, and left four other derivations of
 * `~/.config/biorouter` standing — including the two in the `brxt:uninstall`
 * handler, which feed `fsSync.rmSync(installDir, { recursive: true, force:
 * true })`. A sandboxed dev build could therefore recursively delete out of the
 * developer's real extensions tree. A second resolver "kept in step" is the
 * shape that bug had; there is one function, and its callers pass it on.
 *
 * ## What it must agree with
 *
 * The authority is Rust: `crates/biorouter/src/config/paths.rs`
 * (`Paths::get_dir`), which the daemon, the CLI and every in-process store read
 * through. It resolves in two steps, and both were wrong here:
 *
 * 1. **`BIOROUTER_PATH_ROOT`** — the sandbox seam. `<root>/config`, the shape
 *    `main.ts` already used for `config.yaml`.
 * 2. Otherwise the **platform** directory, via `etcetera`'s
 *    `choose_app_strategy` with the workspace's one `AppStrategyArgs`
 *    (`biorouter-mcp`'s `APP_STRATEGY`, author `UCSF`, app `biorouter`).
 *    That is the **Xdg** strategy on macOS as well as Linux (verified in
 *    etcetera 0.11's `app_strategy.rs`: `create_strategies!(Apple, Xdg)` puts
 *    Xdg in the `choose_app_strategy` slot), and the **Windows** strategy on
 *    Windows, where the folder is the one `./appFolder.ts` resolves: the
 *    current one, or the folder earlier releases used until the daemon has
 *    moved it. It is NOT
 *    `~/.config/biorouter` on either count: a non-default `XDG_CONFIG_HOME`
 *    moves it, and Windows has never used that layout.
 *    `crates/biorouter-server/src/routes/shell.rs` fixed and documented the
 *    same defect on the Rust side.
 *
 * ## The empty-string decision
 *
 * A set-but-blank `BIOROUTER_PATH_ROOT` was read three different ways —
 * `Paths::get_dir` takes it literally and yields a **cwd-relative** `./config`,
 * `biorouter-mcp::resolve_config_dir` treats it as unset, and this module used
 * to ignore the variable entirely. Blank is treated as **unset** here, and the
 * reason is not taste:
 *
 * - A relative config dir has no cross-process meaning. The daemon is spawned
 *   with a working directory of its own, so `./config` names a *different*
 *   directory in the daemon than in the Electron main process, whatever this
 *   module does. Mirroring `Paths` byte-for-byte would therefore NOT restore
 *   agreement — it would point a recursive-delete writer at
 *   `<electron cwd>/config/extensions`, which for a packaged app is not even
 *   inside the user's home.
 * - "Set but empty means unset" is already what every other resolver on both
 *   sides of the boundary does: `biorouter-mcp::resolve_config_dir`,
 *   `routes::shell::home_dir` (`.filter(|p| !p.as_os_str().is_empty())`), and
 *   `main.ts`'s own `expandBiorouterPath`, whose `if (!pathRoot)` has always
 *   made an empty string falsy.
 *
 * ⚠ **This leaves `Paths::get_dir` as the odd one out**, and it is the half
 * that cannot be fixed from here: with the variable exported but empty the
 * daemon still writes `./config`. The fix is one line in `paths.rs`
 * (`std::env::var(…).ok().filter(|root| !root.trim().is_empty())`), and until
 * it lands, an empty `BIOROUTER_PATH_ROOT` is a broken sandbox no matter what
 * this file does — not a reason to point the writers at the real tree.
 */

import * as os from 'os';
import * as path from 'path';
import { APP_NAME, appRootUnder, resolveAppRoot, windowsBases, type Presence } from './appFolder';

const APP_STRATEGY_APP_NAME = APP_NAME;

/** What the platform half of the resolver reads; `process` and `os` in the app. */
export interface PlatformInputs {
  platform: typeof process.platform;
  env: Record<string, string | undefined>;
  homedir: string;
  /** `path.win32` lets the suite run the Windows branch on every OS. */
  pathImpl?: Pick<typeof path, 'join' | 'isAbsolute'>;
  presence?: (target: string) => Presence;
}

/**
 * The platform config directory — what `choose_app_strategy(...).config_dir()`
 * returns when no sandbox root is set. Exported for the suite only, which runs
 * the Windows branch on every OS; the app calls it through
 * `biorouterConfigDir`.
 *
 * Windows: `%APPDATA%\UCSF\biorouter\config`, or the same folder under the
 * legacy author while the daemon has not moved it yet (see `./appFolder.ts`;
 * this process never moves it). ⚠
 * etcetera's ladder has a rung this cannot reproduce — an unset `APPDATA`
 * sends it to the known-folder API before it falls back to
 * `<home>\AppData\Roaming` — so this skips the middle rung and joins the home
 * dir directly. On a normal profile the two name the same directory, and
 * `APPDATA` is set in every Electron process we ship.
 *
 * Elsewhere (macOS included): `$XDG_CONFIG_HOME/biorouter`, or
 * `~/.config/biorouter` when that variable is unset or **not absolute** — the
 * XDG spec says a relative value is ignored, and etcetera implements exactly
 * that (`env_var_or_none` returns `None` unless `path.is_absolute()`).
 */
export function platformConfigDirFor(inputs: PlatformInputs): string {
  const p = inputs.pathImpl ?? path;
  if (inputs.platform === 'win32') {
    const { roaming } = windowsBases(inputs.env, inputs.homedir, p.join);
    const root = appRootUnder(roaming, p.join);
    return p.join(resolveAppRoot(root, inputs.presence), 'config');
  }

  const xdgConfigHome = inputs.env.XDG_CONFIG_HOME;
  const base =
    xdgConfigHome && p.isAbsolute(xdgConfigHome)
      ? xdgConfigHome
      : p.join(inputs.homedir, '.config');
  return p.join(base, APP_STRATEGY_APP_NAME);
}

function platformConfigDir(): string {
  return platformConfigDirFor({
    platform: process.platform,
    env: process.env,
    homedir: os.homedir(),
  });
}

/**
 * Every prefix that names the config directory when NO sandbox root is set.
 *
 * Used by `expandBiorouterPath` to decide which incoming paths get redirected
 * into the sandbox. It is a list, not a single value, because both spellings
 * are in circulation: the platform directory is what Rust resolves, while
 * `~/.config/biorouter` is what hardcoded strings elsewhere in the app still
 * say — and under a non-default `XDG_CONFIG_HOME` those are different
 * directories. Redirecting both is the containing choice; redirecting only the
 * platform one would leave a hardcoded path pointing at the real tree, which is
 * the very thing the sandbox root exists to prevent.
 */
export function unsandboxedConfigDirCandidates(): string[] {
  const candidates = [platformConfigDir()];
  if (process.platform === 'win32') {
    // Both Windows folders are real, whichever one is in use: a path naming
    // the other must be redirected into the sandbox too.
    const root = appRootUnder(windowsBases(process.env, os.homedir()).roaming);
    candidates.push(path.join(root.current, 'config'), path.join(root.legacy, 'config'));
  }
  candidates.push(path.join(os.homedir(), '.config', APP_STRATEGY_APP_NAME));
  return [...new Set(candidates)];
}

/**
 * The Biorouter config directory this process must read and write.
 *
 * Resolved on every call, and never cached at module load. That is robustness,
 * not testability: the value is read long after import (the update check fires
 * on a 15 s timer), and a module-level constant is precisely how #146 froze the
 * answer before any redirect could apply. It costs nothing — a handful of calls
 * per launch, each at most two env reads, an `os.homedir()` and a join.
 */
export function biorouterConfigDir(): string {
  const sandboxRoot = process.env.BIOROUTER_PATH_ROOT;
  // Raw, not trimmed, once we know it is not blank: `Paths` and
  // `resolve_config_dir` both build the path from the value as given, so a root
  // that genuinely begins with a space still resolves the same on both sides.
  if (sandboxRoot && sandboxRoot.trim().length > 0) {
    return path.join(sandboxRoot, 'config');
  }
  return platformConfigDir();
}

/** `<config>/<sub>`, the shape of `Paths::in_config_dir`. Not exported: every
 *  caller so far wants either the config dir itself or the one join below, and
 *  an exported helper nobody calls is how a second resolver starts. */
function inBiorouterConfigDir(...sub: string[]): string {
  return path.join(biorouterConfigDir(), ...sub);
}

/**
 * The `extensions/` directory the updater reads, the `.brxt` handlers write,
 * and `uv sync` runs in — `Paths::config_dir().join("extensions")`, the same
 * join `routes::shell::extensions_dir` makes.
 */
export function biorouterExtensionsDir(): string {
  return inBiorouterConfigDir('extensions');
}
