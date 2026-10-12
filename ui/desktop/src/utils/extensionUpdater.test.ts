// @vitest-environment node

/**
 * Issue #146 — the main process derived Biorouter's config directory in five
 * places, and they did not agree.
 *
 * `extensionUpdater.ts` built `~/.config/biorouter/extensions` into a
 * module-level constant, ignoring `BIOROUTER_PATH_ROOT`. It is not a read-only
 * module — it extracts a downloaded `.brxt` over `installDir` and then runs
 * `uv sync` there — and it fires on a timer 15 s after every launch, so a
 * sandboxed dev build enumerated (and would have rewritten) the developer's
 * nine real extensions.
 *
 * ⚠ Fixing that module alone left the WORSE sites standing: `main.ts`'s
 * `brxt:uninstall` handler derived the same path from `os.homedir()` and handed
 * it to `fsSync.rmSync(dir, { recursive: true, force: true })`, `brxt:install`
 * did the same before extracting an archive into it, and
 * `extensionProvenance.ts` wrote its store there. (`renderer.tsx`'s browser
 * shim had a fifth, `${home_dir}/.config/biorouter`, with no callers at all;
 * it was deleted rather than rewired.) There is now ONE resolver —
 * `utils/biorouterPaths.ts` — and the last describe block below is what stops
 * the next derivation appearing.
 *
 * WHAT EACH TEST CATCHES. Some are behavioural — they drive the real
 * `runExtensionUpdateCheck` against a fake home — and the rest are resolution
 * or source assertions, for the facts no behavioural test in this file can see.
 */
import { describe, it, expect, vi, beforeEach, afterEach, beforeAll } from 'vitest';
import * as fs from 'fs/promises';
import { readFileSync, existsSync } from 'fs';
import * as path from 'path';

/** Reassigned per test; every `os` reader below closes over it. */
let fakeHome: string;

const sentEvents: Array<Record<string, unknown>> = [];

vi.mock('electron', () => ({
  BrowserWindow: {
    getAllWindows: () => [
      {
        isDestroyed: () => false,
        webContents: {
          send: (_channel: string, event: Record<string, unknown>) => {
            sentEvents.push(event);
          },
        },
      },
    ],
  },
}));

vi.mock('./logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() },
}));

// Mocked, not spied: `./dependencyChecker` pulls in `../biorouterd`, i.e. the
// whole Electron main-process graph. Nothing here should reach it anyway — the
// fixture manifest names a non-GitHub repository, so the update check bails
// before any download — and a call landing on this throw would say so loudly.
vi.mock('./dependencyChecker', () => ({
  runProbe: vi.fn(async () => {
    throw new Error('runProbe must not be reached: no update should be attempted');
  }),
}));

// `os` has to be MOCKED, not spied: an ES module namespace is not configurable,
// so `vi.spyOn(os, 'homedir')` throws "Cannot redefine property". Same shape as
// `githubUpdaterDownload.test.ts`.
vi.mock('os', async (importOriginal) => {
  const actual = await importOriginal<typeof import('os')>();
  return { ...actual, default: { ...actual, homedir: () => fakeHome }, homedir: () => fakeHome };
});

const TMP_BASE = process.env.TMPDIR || process.env.TEMP || '/tmp';

/** The suite's platform assumption, stated once. */
const IS_WINDOWS = process.platform === 'win32';

/**
 * Import the module under test fresh, AFTER `fakeHome` and the environment are
 * set. Deliberately dynamic: the pre-fix implementation resolved the directory
 * at module-evaluation time, and a static import would have frozen the answer
 * before any test could move it — so a static import here would let the broken
 * implementation pass the redirect tests by accident.
 */
async function loadUpdater() {
  vi.resetModules();
  return import('./extensionUpdater');
}

/** A minimal installed extension: one directory holding one manifest.json. */
async function seedExtension(extensionsDir: string, name: string) {
  const dir = path.join(extensionsDir, name);
  await fs.mkdir(dir, { recursive: true });
  await fs.writeFile(
    path.join(dir, 'manifest.json'),
    JSON.stringify({
      name,
      display_name: name,
      description: '',
      version: '1.0.0',
      entry_point: 'main.py',
      // NOT github.com: `parseGitHubRepo` returns null, so the check reaches
      // "found an extension" and stops before any network call. The assertion
      // is about WHICH TREE was scanned, and nothing else.
      repository: 'https://git.example.invalid/owner/repo',
    })
  );
  return dir;
}

let originalPathRoot: string | undefined;
let originalXdgConfigHome: string | undefined;
let originalAppData: string | undefined;
let originalLocalAppData: string | undefined;
let redirectRoot: string;

beforeAll(() => {
  originalPathRoot = process.env.BIOROUTER_PATH_ROOT;
  originalXdgConfigHome = process.env.XDG_CONFIG_HOME;
  originalAppData = process.env.APPDATA;
  originalLocalAppData = process.env.LOCALAPPDATA;
});

function restoreEnv(name: string, value: string | undefined) {
  if (value === undefined) delete process.env[name];
  else process.env[name] = value;
}

beforeEach(async () => {
  sentEvents.length = 0;
  delete process.env.BIOROUTER_PATH_ROOT;
  // The platform reading is `$XDG_CONFIG_HOME/biorouter` when that variable is
  // set, so a developer who exports it would otherwise turn every "unset"
  // expectation below red for a reason that has nothing to do with the code.
  delete process.env.XDG_CONFIG_HOME;
  fakeHome = await fs.mkdtemp(path.join(TMP_BASE, 'br-extupd-home-'));
  redirectRoot = await fs.mkdtemp(path.join(TMP_BASE, 'br-extupd-root-'));
  // R-31: on Windows the resolver reads both bases (and the folder earlier
  // releases used under each). Point them into the fake home, so no test here
  // ever looks at the developer's real profile, whatever it unsets.
  process.env.APPDATA = path.join(fakeHome, 'AppData', 'Roaming');
  process.env.LOCALAPPDATA = path.join(fakeHome, 'AppData', 'Local');
});

afterEach(async () => {
  if (originalPathRoot === undefined) delete process.env.BIOROUTER_PATH_ROOT;
  else process.env.BIOROUTER_PATH_ROOT = originalPathRoot;
  if (originalXdgConfigHome === undefined) delete process.env.XDG_CONFIG_HOME;
  else process.env.XDG_CONFIG_HOME = originalXdgConfigHome;
  restoreEnv('APPDATA', originalAppData);
  restoreEnv('LOCALAPPDATA', originalLocalAppData);
  await fs.rm(fakeHome, { recursive: true, force: true });
  await fs.rm(redirectRoot, { recursive: true, force: true });
});

describe('extension updater directory resolution (#146)', () => {
  /**
   * The control. Catches an "always redirect" implementation — one that reads
   * the variable but forgets the unset case, or drops the `.config/biorouter`
   * segment — which every redirect assertion below would happily pass.
   */
  it.skipIf(IS_WINDOWS)(
    'is the real config directory when nothing redirects (macOS/Linux)',
    async () => {
      const { biorouterExtensionsDir } = await loadUpdater();
      expect(biorouterExtensionsDir()).toBe(
        path.join(fakeHome, '.config', 'biorouter', 'extensions')
      );
    }
  );

  /**
   * The defect itself. Fails against the shipped constant, which was
   * `path.join(os.homedir(), '.config', 'biorouter', 'extensions')` regardless
   * of the variable.
   */
  it('moves under BIOROUTER_PATH_ROOT', async () => {
    process.env.BIOROUTER_PATH_ROOT = redirectRoot;
    const { biorouterExtensionsDir } = await loadUpdater();
    expect(biorouterExtensionsDir()).toBe(path.join(redirectRoot, 'config', 'extensions'));
    expect(biorouterExtensionsDir().startsWith(fakeHome)).toBe(false);
  });

  /**
   * Resolution happens per call, not once. Catches a fix that reads the
   * variable but caches the answer in a module-level constant — which would
   * pass both tests above (each gets a fresh module) and still be wrong in the
   * running app, where the check fires long after import.
   */
  it('re-reads the variable on every call', async () => {
    const { biorouterExtensionsDir } = await loadUpdater();
    const before = biorouterExtensionsDir();
    process.env.BIOROUTER_PATH_ROOT = redirectRoot;
    expect(biorouterExtensionsDir()).not.toBe(before);
    expect(biorouterExtensionsDir()).toBe(path.join(redirectRoot, 'config', 'extensions'));
  });

  /**
   * ⚠ This test replaces one that asserted the home-only fallback
   * (`path.join(os.homedir(), '.config', 'biorouter')`, unconditionally). That
   * assertion pinned a divergence: Rust resolves through `choose_app_strategy`,
   * which is etcetera's **Xdg** strategy on macOS as well as Linux, so a
   * non-default `XDG_CONFIG_HOME` puts the daemon's extensions somewhere the
   * old updater never looked. `routes::shell.rs` fixed the same defect on the
   * Rust side.
   *
   * Catches: any resolver that ignores `XDG_CONFIG_HOME` — including the one
   * this branch shipped.
   */
  it.skipIf(IS_WINDOWS)('follows XDG_CONFIG_HOME, as the daemon does', async () => {
    const xdg = await fs.mkdtemp(path.join(TMP_BASE, 'br-extupd-xdg-'));
    try {
      process.env.XDG_CONFIG_HOME = xdg;
      const { biorouterExtensionsDir } = await loadUpdater();
      expect(biorouterExtensionsDir()).toBe(path.join(xdg, 'biorouter', 'extensions'));
    } finally {
      await fs.rm(xdg, { recursive: true, force: true });
    }
  });

  /**
   * The XDG spec says a relative value is ignored, and etcetera implements
   * exactly that (`env_var_or_none` returns `None` unless `path.is_absolute()`).
   *
   * Catches: an `if (xdg)` that trusts any non-empty string — which would
   * resolve the extensions tree relative to the Electron process's working
   * directory, a place the daemon has never heard of.
   */
  it.skipIf(IS_WINDOWS)('ignores a relative XDG_CONFIG_HOME, as etcetera does', async () => {
    process.env.XDG_CONFIG_HOME = 'relative/config';
    const { biorouterExtensionsDir } = await loadUpdater();
    expect(biorouterExtensionsDir()).toBe(
      path.join(fakeHome, '.config', 'biorouter', 'extensions')
    );
  });

  /**
   * Windows never used the `~/.config/biorouter` layout at all: `Paths`
   * resolves through etcetera's Windows strategy, i.e.
   * `%APPDATA%\<author>\<app>\config`. Runs only on Windows so the expectation
   * can stay a literal rather than a re-derivation of the code under test.
   */
  it.runIf(IS_WINDOWS)('uses the Windows layout on Windows', async () => {
    const appData = await fs.mkdtemp(path.join(TMP_BASE, 'br-extupd-appdata-'));
    try {
      process.env.APPDATA = appData;
      const { biorouterExtensionsDir } = await loadUpdater();
      expect(biorouterExtensionsDir()).toBe(
        path.join(appData, 'UCSF', 'biorouter', 'config', 'extensions')
      );
    } finally {
      await fs.rm(appData, { recursive: true, force: true });
    }
  });

  /**
   * R-31. Resolving never touches either Windows base: the folder earlier
   * releases used stays exactly where it is, and nothing is created beside
   * it. (The desktop used to move it on the first resolution, so this suite
   * moved a developer's real profile on Windows.) Runs on every OS through
   * the Windows branch the suite can reach, `platformConfigDirFor`, against
   * real directories.
   */
  it('resolving the Windows layout moves nothing', async () => {
    const roaming = path.join(fakeHome, 'AppData', 'Roaming');
    const { APP_AUTHOR, APP_NAME, LEGACY_APP_AUTHOR } = await import('./appFolder');
    const legacyConfig = path.join(roaming, LEGACY_APP_AUTHOR, APP_NAME, 'config');
    await fs.mkdir(legacyConfig, { recursive: true });
    const { platformConfigDirFor } = await import('./biorouterPaths');
    expect(platformConfigDirFor({ platform: 'win32', env: process.env, homedir: fakeHome })).toBe(
      legacyConfig
    );
    expect(existsSync(legacyConfig)).toBe(true);
    expect(existsSync(path.join(roaming, APP_AUTHOR))).toBe(false);
  });

  /**
   * The empty-string decision, and the reason it is not cosmetic.
   *
   * `BIOROUTER_PATH_ROOT` was read three different ways: `Paths::get_dir` takes
   * a blank value literally and yields a **cwd-relative** `./config`,
   * `biorouter-mcp::resolve_config_dir` treats it as unset, and this module
   * ignored the variable outright. Blank means unset here — a relative config
   * dir has no cross-process meaning (the daemon has its own working
   * directory), so mirroring `Paths` byte-for-byte would not restore agreement,
   * it would point a recursive-delete writer at `<electron cwd>/config`.
   *
   * Catches: a truthiness check that lets `'   '` through — the plausible
   * implementation, which every other test in this block passes.
   */
  it.skipIf(IS_WINDOWS)('reads a blank BIOROUTER_PATH_ROOT as unset', async () => {
    const homeExtensions = path.join(fakeHome, '.config', 'biorouter', 'extensions');
    for (const blank of ['', '   ', '\t']) {
      process.env.BIOROUTER_PATH_ROOT = blank;
      const { biorouterExtensionsDir } = await loadUpdater();
      expect(biorouterExtensionsDir()).toBe(homeExtensions);
    }
  });

  /**
   * A root that merely *contains* spaces is a real directory and must still
   * redirect. Catches an over-eager fix that trims the value away, or treats
   * any whitespace as blank.
   */
  it('still redirects for a root with spaces in it', async () => {
    const spaced = path.join(redirectRoot, 'a root with spaces');
    await fs.mkdir(spaced, { recursive: true });
    process.env.BIOROUTER_PATH_ROOT = spaced;
    const { biorouterExtensionsDir } = await loadUpdater();
    expect(biorouterExtensionsDir()).toBe(path.join(spaced, 'config', 'extensions'));
  });
});

/**
 * The behavioural half — the bug as the user met it, driven through the real
 * exported entry point. Posix-only: these seed a fixture at the `~/.config`
 * layout, which is not where Windows resolves; the Windows layout is pinned by
 * its own resolution test above rather than by a second copy of this suite.
 */
describe.skipIf(IS_WINDOWS)('the update check reads the redirected tree, not home (#146)', () => {
  /**
   * `runExtensionUpdateCheck` returns early, emitting NOTHING, when it finds no
   * extensions; it emits `all-done` once it has found at least one. So the
   * events are a direct readout of which directory was scanned. With the
   * pre-fix constant this test goes red: home is empty, so the run bails and
   * `sentEvents` stays empty.
   */
  it('finds an extension that exists only under the redirected root', async () => {
    process.env.BIOROUTER_PATH_ROOT = redirectRoot;
    await seedExtension(path.join(redirectRoot, 'config', 'extensions'), 'redirected-ext');

    const { runExtensionUpdateCheck } = await loadUpdater();
    await runExtensionUpdateCheck();

    expect(sentEvents).toEqual([{ type: 'all-done', updatedCount: 0 }]);
  });

  /**
   * The other direction, and the one that is actually the security claim: with
   * the redirect set, an extension sitting in the developer's REAL home must
   * not be enumerated. Catches a fix that adds the redirected directory as an
   * extra scan root instead of replacing home — which would pass the test above
   * while leaving the developer's nine extensions exactly as exposed.
   */
  it('does not enumerate home while redirected', async () => {
    await seedExtension(path.join(fakeHome, '.config', 'biorouter', 'extensions'), 'home-ext');
    process.env.BIOROUTER_PATH_ROOT = redirectRoot;
    await fs.mkdir(path.join(redirectRoot, 'config', 'extensions'), { recursive: true });

    const { runExtensionUpdateCheck } = await loadUpdater();
    await runExtensionUpdateCheck();

    expect(sentEvents).toEqual([]);
  });

  /**
   * The instrument's own control: the same home fixture IS found when nothing
   * redirects. Without this, the assertion above would pass for a
   * `runExtensionUpdateCheck` that had simply stopped working.
   */
  it('still enumerates home when nothing redirects', async () => {
    await seedExtension(path.join(fakeHome, '.config', 'biorouter', 'extensions'), 'home-ext');

    const { runExtensionUpdateCheck } = await loadUpdater();
    await runExtensionUpdateCheck();

    expect(sentEvents).toEqual([{ type: 'all-done', updatedCount: 0 }]);
  });
});

/**
 * Source assertions, for the one fact no behavioural test in this file can see:
 * WHICH modules do the deriving. `main.ts` cannot be imported under vitest at
 * all, so its agreement with this module is only checkable against the shipped
 * source. This follows `workspaceChannelCsp.test.ts`'s precedent and carries
 * its caveat — reading source text pins the coupling, not the behaviour. The
 * behaviour is pinned above.
 */
describe('one resolver, and the main process reads it (#146)', () => {
  /** `<repo>/ui/desktop/src`, from wherever vitest was started. */
  function desktopSrcDir(): string {
    const candidates = [
      path.join(process.cwd(), 'src', 'main.ts'),
      path.join(process.cwd(), 'ui', 'desktop', 'src', 'main.ts'),
    ];
    const mainPath = candidates.find((p) => existsSync(p));
    if (!mainPath) throw new Error(`could not locate src/main.ts from ${process.cwd()}`);
    return path.dirname(mainPath);
  }

  /**
   * Source with comment lines removed, so a comment *describing* the old
   * hardcoded path — this file's own history is full of them, and so are the
   * modules under test — cannot be mistaken for the code that did it.
   */
  function codeOf(file: string): string {
    return readFileSync(path.join(desktopSrcDir(), file), 'utf-8')
      .split('\n')
      .filter((line) => {
        const trimmed = line.trim();
        return !trimmed.startsWith('//') && !trimmed.startsWith('*') && !trimmed.startsWith('/*');
      })
      .join('\n');
  }

  /**
   * The rule the whole fix rests on. Every one of these modules used to build
   * the config directory itself; `main.ts` did it three times, twice inside
   * `.brxt` handlers that create, extract into, and recursively delete that
   * directory.
   *
   * Catches: a sixth derivation — including the plausible "just this one place,
   * it's only a fallback" — which no behavioural test above would see, because
   * the site that regresses is `brxt:uninstall`, and nothing in this suite can
   * invoke an Electron IPC handler.
   */
  it('no main-process module derives the config directory itself', () => {
    for (const file of [
      'main.ts',
      'renderer.tsx',
      'utils/extensionUpdater.ts',
      'utils/extensionProvenance.ts',
    ]) {
      const code = codeOf(file);
      expect(code, `${file} joins a config dir onto the home directory`).not.toMatch(
        /homedir\(\)\s*,\s*['"]\.config['"]/
      );
      expect(code, `${file} hardcodes the config directory as a string`).not.toMatch(
        /\.config\/biorouter/
      );
    }
  });

  /**
   * …and the modules that need it import the one resolver. Catches the other
   * half of the same regression: deleting a derivation without replacing it,
   * leaving a module that silently no longer resolves anything.
   */
  it('the modules that need a config directory import the shared resolver', () => {
    for (const file of ['main.ts', 'utils/extensionUpdater.ts', 'utils/extensionProvenance.ts']) {
      expect(codeOf(file), `${file} does not import ./biorouterPaths`).toMatch(
        /from '\.{1,2}\/(utils\/)?biorouterPaths'/
      );
    }
  });

  /**
   * The redirect shape (`<root>/config`, not `<root>` and not `<root>/.config`)
   * is the daemon's, and now lives in exactly one place. Catches the plausible
   * wrong fix of honouring the variable with a different layout —
   * `path.join(pathRoot, 'extensions')`, say — which resolves to a real
   * directory, passes every assertion above, and puts the updater in a tree the
   * daemon and CLI have never heard of.
   */
  it('the shared resolver keeps the redirect shape <BIOROUTER_PATH_ROOT>/config', () => {
    const resolver = codeOf('utils/biorouterPaths.ts');
    expect(resolver).toContain('process.env.BIOROUTER_PATH_ROOT');
    expect(resolver).toMatch(/path\.join\(sandboxRoot, 'config'\)/);
  });
});
