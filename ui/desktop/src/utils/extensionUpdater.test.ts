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
import { setImmediate as nextEventLoopTurn } from 'node:timers/promises';
import AdmZip from 'adm-zip';
import { runProbe } from './dependencyChecker';

const immediateFilesystem = vi.hoisted(() => ({ enabled: false }));

vi.mock('fs/promises', async (importOriginal) => {
  const actual = await importOriginal<typeof import('fs/promises')>();
  const synchronous = await import('fs');
  const methods = ['cp', 'mkdir', 'mkdtemp', 'readFile', 'readdir', 'rename', 'rm', 'writeFile'];
  const wrapped = Object.fromEntries(
    methods.map((name) => [
      name,
      (...args: unknown[]) => {
        if (!immediateFilesystem.enabled)
          return Reflect.apply(Reflect.get(actual, name), actual, args);
        // Real fixture I/O, with no worker-pool timing in the overlap schedule.
        try {
          return Promise.resolve(
            Reflect.apply(Reflect.get(synchronous, `${name}Sync`), synchronous, args)
          );
        } catch (error) {
          return Promise.reject(error);
        }
      },
    ])
  );
  return { ...actual, ...wrapped };
});

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
  immediateFilesystem.enabled = false;
  sentEvents.length = 0;
  vi.mocked(runProbe).mockReset().mockRejectedValue(new Error('unexpected dependency probe'));
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
  immediateFilesystem.enabled = false;
  vi.unstubAllGlobals();
  if (originalPathRoot === undefined) delete process.env.BIOROUTER_PATH_ROOT;
  else process.env.BIOROUTER_PATH_ROOT = originalPathRoot;
  if (originalXdgConfigHome === undefined) delete process.env.XDG_CONFIG_HOME;
  else process.env.XDG_CONFIG_HOME = originalXdgConfigHome;
  restoreEnv('APPDATA', originalAppData);
  restoreEnv('LOCALAPPDATA', originalLocalAppData);
  await fs.rm(fakeHome, { recursive: true, force: true });
  await fs.rm(redirectRoot, { recursive: true, force: true });
});

describe('extension update commit reporting and rollback', () => {
  const successfulProbe = {
    ok: true,
    code: 0,
    stdout: '',
    stderr: '',
    timedOut: false,
    error: null,
  };

  const failedProbe = {
    ...successfulProbe,
    ok: false,
    code: 19,
    stderr: 'synthetic sync failure',
  };

  async function installedPackage(name: string) {
    process.env.BIOROUTER_PATH_ROOT = redirectRoot;
    const dir = await seedExtension(path.join(redirectRoot, 'config', 'extensions'), name);
    const manifest = JSON.parse(await fs.readFile(path.join(dir, 'manifest.json'), 'utf8'));
    manifest.repository = `https://github.com/l7-fixtures/${name}`;
    await fs.writeFile(path.join(dir, 'manifest.json'), JSON.stringify(manifest));
    await fs.writeFile(path.join(dir, 'main.py'), 'working version 1\n');
    await fs.mkdir(path.join(dir, '.venv', 'bin'), { recursive: true });
    await fs.writeFile(path.join(dir, '.venv', 'bin', 'fixture'), 'working runtime 1\n');
    return { name, dir, manifest };
  }

  function bundle(manifest: Record<string, unknown>, partialExtraction = false, version = '2.0.0') {
    const zip = new AdmZip();
    zip.addFile('manifest.json', Buffer.from(JSON.stringify({ ...manifest, version })));
    zip.addFile('main.py', Buffer.from(`replacement version ${version.split('.')[0]}\n`));
    if (partialExtraction) {
      zip.addFile('z-conflict', Buffer.from('file blocks a later parent directory'));
      zip.addFile('z-conflict/child', Buffer.from('cannot extract'));
    }
    return zip.toBuffer();
  }

  function releases(
    packages: Array<{ name: string; manifest: Record<string, unknown> }>,
    partial = false
  ) {
    const downloads = new Map(packages.map((pkg) => [pkg.name, bundle(pkg.manifest, partial)]));
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: string | URL | Request) => {
        const url = String(input);
        const pkg = packages.find(({ name }) => url.includes(`/${name}/`));
        if (!pkg) throw new Error(`unexpected fixture request: ${url}`);
        if (url.endsWith('/releases/latest')) {
          return new Response(
            JSON.stringify({
              tag_name: 'v2.0.0',
              assets: [
                {
                  name: `${pkg.name}.brxt`,
                  browser_download_url: `https://downloads.example.invalid/${pkg.name}/fixture.brxt`,
                  size: downloads.get(pkg.name)!.length,
                },
              ],
            }),
            { status: 200 }
          );
        }
        const bytes = downloads.get(pkg.name)!;
        return new Response(new Uint8Array(bytes), {
          status: 200,
          headers: { 'content-length': String(bytes.length) },
        });
      })
    );
  }

  async function packageBytes(root: string): Promise<Record<string, string>> {
    const files: Record<string, string> = {};
    async function visit(dir: string) {
      for (const entry of await fs.readdir(dir, { withFileTypes: true })) {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) await visit(full);
        else files[path.relative(root, full)] = (await fs.readFile(full)).toString('base64');
      }
    }
    await visit(root);
    return files;
  }

  it('counts no successful update after uv sync fails', async () => {
    const pkg = await installedPackage('l7-failed-count');
    releases([pkg]);
    vi.mocked(runProbe).mockResolvedValue(failedProbe);
    const { runExtensionUpdateCheck } = await loadUpdater();

    await runExtensionUpdateCheck();

    expect(sentEvents).toContainEqual(
      expect.objectContaining({ type: 'update-error', ext: pkg.name })
    );
    expect(sentEvents.filter((event) => event.type === 'update-done')).toEqual([]);
    expect(sentEvents[sentEvents.length - 1]).toEqual({ type: 'all-done', updatedCount: 0 });
  });

  it('restores the complete working package after a sync changes its runtime and fails', async () => {
    const pkg = await installedPackage('l7-failed-files');
    const before = await packageBytes(pkg.dir);
    releases([pkg]);
    vi.mocked(runProbe).mockImplementation(async (_command, _args, _timeout, options) => {
      await fs.mkdir(path.join(options!.cwd!, '.venv', 'bin'), { recursive: true });
      await fs.writeFile(
        path.join(options!.cwd!, '.venv', 'bin', 'fixture'),
        'broken replacement runtime\n'
      );
      return failedProbe;
    });
    const { runExtensionUpdateCheck } = await loadUpdater();

    await runExtensionUpdateCheck();

    expect(await packageBytes(pkg.dir)).toEqual(before);
    expect(await fs.readdir(path.dirname(pkg.dir))).toEqual([pkg.name]);
  });

  it('restores the working package if extraction fails after writing replacement files', async () => {
    const pkg = await installedPackage('l7-partial-extraction');
    const before = await packageBytes(pkg.dir);
    releases([pkg], true);
    const { runExtensionUpdateCheck } = await loadUpdater();

    await runExtensionUpdateCheck();

    expect(sentEvents).toContainEqual(
      expect.objectContaining({ type: 'update-error', ext: pkg.name })
    );
    expect(runProbe).not.toHaveBeenCalled();
    expect(await packageBytes(pkg.dir)).toEqual(before);
    expect(sentEvents[sentEvents.length - 1]).toEqual({ type: 'all-done', updatedCount: 0 });
  });

  it('counts only the committed package in a mixed success and failure check', async () => {
    const failed = await installedPackage('l7-mixed-failed');
    const succeeded = await installedPackage('l7-mixed-success');
    releases([failed, succeeded]);
    vi.mocked(runProbe).mockImplementation(async (_command, _args, _timeout, options) =>
      options!.cwd === succeeded.dir ? successfulProbe : failedProbe
    );
    const { runExtensionUpdateCheck } = await loadUpdater();

    await runExtensionUpdateCheck();

    expect(sentEvents.filter((event) => event.type === 'update-done')).toEqual([
      expect.objectContaining({ type: 'update-done', ext: succeeded.name }),
    ]);
    expect(sentEvents[sentEvents.length - 1]).toEqual({ type: 'all-done', updatedCount: 1 });
    expect(await fs.readFile(path.join(failed.dir, 'main.py'), 'utf8')).toBe('working version 1\n');
    expect(await fs.readFile(path.join(succeeded.dir, 'main.py'), 'utf8')).toBe(
      'replacement version 2\n'
    );
  });

  it('commits a successful update with its runtime built at the stable install path', async () => {
    const pkg = await installedPackage('l7-stable-runtime');
    releases([pkg]);
    vi.mocked(runProbe).mockImplementation(async (_command, _args, _timeout, options) => {
      expect(options!.cwd).toBe(pkg.dir);
      await fs.writeFile(
        path.join(options!.cwd!, '.venv', 'bin', 'fixture'),
        `#!${options!.cwd}/.venv/bin/python\n`
      );
      return successfulProbe;
    });
    const { runExtensionUpdateCheck } = await loadUpdater();

    await runExtensionUpdateCheck();

    expect(sentEvents.filter((event) => event.type === 'update-error')).toEqual([]);
    expect(sentEvents[sentEvents.length - 1]).toEqual({ type: 'all-done', updatedCount: 1 });
    expect(await fs.readFile(path.join(pkg.dir, '.venv', 'bin', 'fixture'), 'utf8')).toBe(
      `#!${pkg.dir}/.venv/bin/python\n`
    );
    expect(await fs.readdir(path.dirname(pkg.dir))).toEqual([pkg.name]);
  });

  it('keeps a committed update when an overlapping older check fails', async () => {
    immediateFilesystem.enabled = true;
    const pkg = await installedPackage('l7-overlapping-checks');
    function deferred<T>() {
      let resolve!: (value: T) => void;
      const promise = new Promise<T>((done) => {
        resolve = done;
      });
      return { promise, resolve };
    }
    const firstProbeEntered = deferred<void>();
    const finishFirstProbe = deferred<typeof failedProbe>();
    const bundles = new Map([
      ['2.0.0', bundle(pkg.manifest, false, '2.0.0')],
      ['3.0.0', bundle(pkg.manifest, false, '3.0.0')],
    ]);
    let latestVersion = '2.0.0';
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: string | URL | Request) => {
        const url = String(input);
        if (url === `https://api.github.com/repos/l7-fixtures/${pkg.name}/releases/latest`) {
          return new Response(
            JSON.stringify({
              tag_name: latestVersion,
              assets: [
                {
                  name: `${pkg.name}.brxt`,
                  browser_download_url: `https://downloads.example.invalid/${pkg.name}/${latestVersion}.brxt`,
                  size: bundles.get(latestVersion)!.length,
                },
              ],
            }),
            { status: 200 }
          );
        }
        const version = url.match(/\/(2\.0\.0|3\.0\.0)\.brxt$/)?.[1];
        if (!version) throw new Error(`unexpected fixture request: ${url}`);
        return new Response(new Uint8Array(bundles.get(version)!), { status: 200 });
      })
    );
    let probes = 0;
    vi.mocked(runProbe).mockImplementation(async (_command, _args, _timeout, options) => {
      probes++;
      if (probes === 1) {
        firstProbeEntered.resolve();
        return finishFirstProbe.promise;
      }
      const manifest = JSON.parse(
        await fs.readFile(path.join(options!.cwd!, 'manifest.json'), 'utf8')
      );
      await fs.mkdir(path.join(options!.cwd!, '.venv', 'bin'), { recursive: true });
      await fs.writeFile(
        path.join(options!.cwd!, '.venv', 'bin', 'fixture'),
        `runtime-${manifest.version}\n`
      );
      return successfulProbe;
    });
    const { runExtensionUpdateCheck } = await loadUpdater();
    const firstCheck = runExtensionUpdateCheck();
    let secondCheck: Promise<void> | undefined;
    try {
      await Promise.race([
        firstProbeEntered.promise,
        firstCheck.then(() => {
          throw new Error('first check ended before reaching its dependency probe');
        }),
      ]);
      latestVersion = '3.0.0';
      secondCheck = runExtensionUpdateCheck();
      // Drain runnable work without waiting for a second probe: a queued or
      // coalesced implementation must be allowed to wait for the first check.
      await nextEventLoopTurn();
      finishFirstProbe.resolve(failedProbe);
      await Promise.all([firstCheck, secondCheck]);

      const committed = sentEvents.filter((event) => event.type === 'update-done');
      if (committed.length > 0) {
        const current = JSON.parse(await fs.readFile(path.join(pkg.dir, 'manifest.json'), 'utf8'));
        expect(current.version, 'a failed check reverted a reported successful update').toBe(
          committed[committed.length - 1].to
        );
      } else {
        // Coalescing may share the first failure. A fresh retry supplies the
        // positive success oracle instead of requiring a second queued check.
        await runExtensionUpdateCheck();
      }

      const current = JSON.parse(await fs.readFile(path.join(pkg.dir, 'manifest.json'), 'utf8'));
      expect(current.version).toBe('3.0.0');
      expect(await fs.readFile(path.join(pkg.dir, 'main.py'), 'utf8')).toBe(
        'replacement version 3\n'
      );
      expect(await fs.readFile(path.join(pkg.dir, '.venv', 'bin', 'fixture'), 'utf8')).toBe(
        'runtime-3.0.0\n'
      );
      expect(sentEvents.filter((event) => event.type === 'update-done')).toEqual([
        expect.objectContaining({ ext: pkg.name, to: '3.0.0' }),
      ]);
      expect(sentEvents.filter((event) => event.type === 'update-error')).toHaveLength(1);
      expect(
        sentEvents
          .filter((event) => event.type === 'all-done')
          .reduce((count, event) => count + Number(event.updatedCount), 0)
      ).toBe(1);
      expect(await fs.readdir(path.dirname(pkg.dir))).toEqual([pkg.name]);
    } finally {
      finishFirstProbe.resolve(failedProbe);
      await Promise.allSettled([firstCheck, ...(secondCheck ? [secondCheck] : [])]);
    }
  });
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
