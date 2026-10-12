// @vitest-environment node

/**
 * The Windows app folder (R-31) from the desktop main process's side.
 *
 * The first block runs the `resolution` cases of the table the daemon's suite
 * runs too (`crates/biorouter-mcp/src/app_folder/resolution-cases.json`),
 * against real temporary directories, so the two implementations resolve
 * every state to the same folder rather than each agreeing with its own idea
 * of it. The `moves` cases are the daemon's alone: this process never moves
 * anything, and the block after the table holds it to that. The rest pins what
 * only this side has: the notice for a hidden legacy folder, the inputs it
 * reads, and the resolver's Windows branch, which runs here on every OS
 * through `path.win32`.
 */
import { afterEach, describe, expect, it } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as appFolder from './appFolder';
import {
  APP_AUTHOR,
  APP_NAME,
  LEGACY_APP_AUTHOR,
  appRootUnder,
  describeLegacyFolderNotice,
  legacyFolderNotice,
  presenceOf,
  resolveAppRoot,
  windowsBases,
  type Presence,
} from './appFolder';
import { platformConfigDirFor } from './biorouterPaths';

interface ResolutionCase {
  name: string;
  before: string[];
  unreadable?: string[];
  resolves: string;
}

const TABLE = new URL(
  '../../../../crates/biorouter-mcp/src/app_folder/resolution-cases.json',
  import.meta.url
);

const temporary: string[] = [];
const locked: string[] = [];
function tempBase(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'br-app-folder-'));
  temporary.push(dir);
  return dir;
}
afterEach(() => {
  for (const dir of locked.splice(0)) fs.chmodSync(dir, 0o755);
  for (const dir of temporary.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});

/** `<base>/<token>/<rest>`, for the tokens the shared table defines. */
function tokenPath(base: string, spec: string): string {
  const [head, ...rest] = spec.replace(/\/+$/, '').split('/');
  const roots: Record<string, string> = {
    current: path.join(base, APP_AUTHOR, APP_NAME),
    legacy: path.join(base, LEGACY_APP_AUTHOR, APP_NAME),
    currentParent: path.join(base, APP_AUTHOR),
    legacyParent: path.join(base, LEGACY_APP_AUTHOR),
    lookalike: path.join(base, `${LEGACY_APP_AUTHOR}2`, APP_NAME),
  };
  const root = roots[head];
  if (!root) throw new Error(`unknown path token ${head}`);
  return path.join(root, ...rest.filter(Boolean));
}

function materialize(base: string, spec: string): void {
  const target = tokenPath(base, spec);
  if (spec.endsWith('/')) {
    fs.mkdirSync(target, { recursive: true });
  } else {
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, spec);
  }
}

/** Remove the permissions of `specs`; false when that cannot set the state up
 *  here (Windows, or permissions not enforced because this runs as root). */
function makeUnreadable(base: string, specs: string[]): boolean {
  if (specs.length === 0) return true;
  if (process.platform === 'win32') return false;
  for (const spec of specs) {
    const dir = tokenPath(base, spec);
    fs.chmodSync(dir, 0o000);
    locked.push(dir);
  }
  return specs.every((spec) => {
    try {
      fs.readdirSync(tokenPath(base, spec));
      return false;
    } catch {
      return true;
    }
  });
}

describe('the resolution table both implementations run', () => {
  const cases: ResolutionCase[] = JSON.parse(fs.readFileSync(TABLE, 'utf-8')).resolution;

  it('has every case the daemon suite counts on', () => {
    expect(cases.length).toBeGreaterThanOrEqual(8);
  });

  for (const testCase of cases) {
    it(testCase.name, (context) => {
      const base = tempBase();
      for (const spec of testCase.before) materialize(base, spec);
      if (!makeUnreadable(base, testCase.unreadable ?? [])) {
        context.skip();
        return;
      }
      expect(resolveAppRoot(appRootUnder(base))).toBe(tokenPath(base, testCase.resolves));
    });
  }
});

describe('this process never moves the folder', () => {
  it('exports nothing that moves, and resolving leaves a legacy folder where it is', () => {
    const exported = Object.keys(appFolder);
    for (const mover of ['moveForProcess', 'moveLegacyRoot', 'ensureAppFolderMoved']) {
      expect(exported).not.toContain(mover);
    }
    const base = tempBase();
    materialize(base, 'legacy/config/config.yaml');
    expect(resolveAppRoot(appRootUnder(base))).toBe(tokenPath(base, 'legacy'));
    expect(fs.existsSync(tokenPath(base, 'legacy/config/config.yaml'))).toBe(true);
    expect(fs.existsSync(tokenPath(base, 'currentParent'))).toBe(false);
  });

  it('the main process does not move it at load either', () => {
    const main = fs.readFileSync(new URL('../main.ts', import.meta.url), 'utf-8');
    expect(main).not.toMatch(/ensureAppFolderMoved|moveForProcess|describeFolderMove/);
    const resolver = fs.readFileSync(new URL('./biorouterPaths.ts', import.meta.url), 'utf-8');
    expect(resolver).not.toMatch(/ensureAppFolderMoved|moveForProcess/);
  });
});

describe('presence', () => {
  it('reads a missing path and a path under a file as absent, and a real one as present', () => {
    const base = tempBase();
    materialize(base, 'legacyParent');
    expect(presenceOf(tokenPath(base, 'legacy'))).toBe('absent');
    expect(presenceOf(tokenPath(base, 'current'))).toBe('absent');
    expect(presenceOf(base)).toBe('present');
  });

  it.skipIf(process.platform === 'win32')(
    'reads a failed check as unknown, not absent',
    (context) => {
      const base = tempBase();
      materialize(base, 'legacy/config/config.yaml');
      if (!makeUnreadable(base, ['legacyParent'])) {
        context.skip();
        return;
      }
      expect(presenceOf(tokenPath(base, 'legacy'))).toBe('unknown');
    }
  );
});

describe('the notice for a hidden legacy folder', () => {
  const both = () => {
    const roaming = tempBase();
    materialize(roaming, 'legacy/config/config.yaml');
    materialize(roaming, 'current/config/config.yaml');
    return roaming;
  };

  it('names both folders when both exist on Windows', () => {
    const roaming = both();
    const notice = legacyFolderNotice({
      platform: 'win32',
      env: { APPDATA: roaming, LOCALAPPDATA: tempBase() },
      homedir: roaming,
    });
    expect(notice).toEqual({
      current: tokenPath(roaming, 'current'),
      legacy: tokenPath(roaming, 'legacy'),
    });
    const words = describeLegacyFolderNotice(notice!);
    expect(words.detail).toContain(tokenPath(roaming, 'legacy'));
    expect(words.detail).toContain(tokenPath(roaming, 'current'));
    expect(words.buttons).toHaveLength(2);
  });

  it('says nothing in every other state, off Windows, and under a sandbox root', () => {
    const states: string[][] = [
      [],
      ['legacy/config/config.yaml'],
      ['current/config/config.yaml'],
      ['lookalike/config/config.yaml', 'current/config/config.yaml'],
    ];
    for (const before of states) {
      const roaming = tempBase();
      for (const spec of before) materialize(roaming, spec);
      expect(
        legacyFolderNotice({ platform: 'win32', env: { APPDATA: roaming }, homedir: roaming }),
        before.join(', ')
      ).toBeUndefined();
    }
    const roaming = both();
    expect(
      legacyFolderNotice({ platform: 'darwin', env: { APPDATA: roaming }, homedir: roaming })
    ).toBeUndefined();
    expect(
      legacyFolderNotice({
        platform: 'win32',
        env: { APPDATA: roaming, BIOROUTER_PATH_ROOT: '/sb' },
        homedir: roaming,
      })
    ).toBeUndefined();
    expect(
      legacyFolderNotice({
        platform: 'win32',
        env: { APPDATA: roaming, BIOROUTER_PATH_ROOT: '   ' },
        homedir: roaming,
      }),
      'a blank root is no root'
    ).toBeDefined();
  });

  it('only for the roaming folder: a cache folder holds nothing to recover', () => {
    const roaming = tempBase();
    materialize(roaming, 'current/config/config.yaml');
    const local = tempBase();
    materialize(local, 'legacy/cache/a.html');
    materialize(local, 'current/cache/a.html');
    expect(
      legacyFolderNotice({
        platform: 'win32',
        env: { APPDATA: roaming, LOCALAPPDATA: local },
        homedir: roaming,
      })
    ).toBeUndefined();
  });

  it('is not raised by a check that failed', () => {
    const roaming = both();
    const unknownLegacy = (target: string): Presence =>
      target === tokenPath(roaming, 'legacy') ? 'unknown' : presenceOf(target);
    expect(
      legacyFolderNotice({
        platform: 'win32',
        env: { APPDATA: roaming },
        homedir: roaming,
        presence: unknownLegacy,
      })
    ).toBeUndefined();
  });
});

describe('the Windows bases', () => {
  it('falls back to the home folder when APPDATA and LOCALAPPDATA are blank', () => {
    const home = 'C:\\Users\\Ana';
    expect(windowsBases({ APPDATA: '', LOCALAPPDATA: '' }, home, path.win32.join)).toEqual({
      roaming: 'C:\\Users\\Ana\\AppData\\Roaming',
      local: 'C:\\Users\\Ana\\AppData\\Local',
    });
  });
});

/**
 * `biorouterConfigDir`'s Windows branch, on any OS: the folder the daemon's
 * `Paths::config_dir()` names for the same state (pinned on the Rust side by
 * `windows_dirs` and the shared table).
 */
describe('the resolver on Windows', () => {
  const home = 'C:\\Users\\Ana';
  const appData = 'C:\\Users\\Ana\\AppData\\Roaming';
  const current = path.win32.join(appData, APP_AUTHOR, APP_NAME);
  const legacy = path.win32.join(appData, LEGACY_APP_AUTHOR, APP_NAME);
  const resolveWith = (states: Record<string, Presence>, env: Record<string, string | undefined>) =>
    platformConfigDirFor({
      platform: 'win32',
      env,
      homedir: home,
      pathImpl: path.win32,
      presence: (target) => states[target] ?? 'absent',
    });

  it('names the current folder on a fresh install', () => {
    expect(resolveWith({}, { APPDATA: appData })).toBe(
      'C:\\Users\\Ana\\AppData\\Roaming\\UCSF\\biorouter\\config'
    );
  });

  it('keeps the legacy folder while the daemon has not moved it', () => {
    expect(resolveWith({ [legacy]: 'present' }, { APPDATA: appData })).toBe(
      path.win32.join(legacy, 'config')
    );
  });

  it('keeps the legacy folder while it cannot be checked', () => {
    expect(resolveWith({ [legacy]: 'unknown' }, { APPDATA: appData })).toBe(
      path.win32.join(legacy, 'config')
    );
  });

  it('prefers the current folder when both exist', () => {
    expect(resolveWith({ [legacy]: 'present', [current]: 'present' }, { APPDATA: appData })).toBe(
      path.win32.join(current, 'config')
    );
  });

  it('joins the home folder when APPDATA is blank, as etcetera does', () => {
    expect(resolveWith({}, { APPDATA: '' })).toBe(
      'C:\\Users\\Ana\\AppData\\Roaming\\UCSF\\biorouter\\config'
    );
  });

  it('never reads the author off Windows', () => {
    expect(
      platformConfigDirFor({
        platform: 'linux',
        env: {},
        homedir: '/home/ana',
        pathImpl: path.posix,
      })
    ).toBe('/home/ana/.config/biorouter');
  });
});
