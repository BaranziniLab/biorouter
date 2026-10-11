// @vitest-environment node

/**
 * The Windows app-folder move (R-31) from the desktop main process's side.
 *
 * The first block runs the table the daemon's suite runs too
 * (`crates/biorouter-mcp/src/app_folder/resolution-cases.json`), against real
 * temporary directories, so the two implementations agree on every state
 * rather than each agreeing with its own idea of it. The rest pins what only
 * this side has: the process-level guards, the inputs it reads, and the
 * resolver's Windows branch, which runs here on every OS through `path.win32`.
 */
import { afterEach, describe, expect, it } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
  APP_AUTHOR,
  APP_NAME,
  LEGACY_APP_AUTHOR,
  MOVE_MARKER,
  NODE_FOLDER_IO,
  appRootUnder,
  describeFolderMove,
  moveForProcess,
  moveLegacyRoot,
  resolveAppRoot,
  type FolderIo,
} from './appFolder';
import { platformConfigDirFor } from './biorouterPaths';

interface Case {
  name: string;
  before: string[];
  renameFails?: boolean;
  runs: string[];
  resolves: string;
  exists: string[];
  absent: string[];
}

const TABLE = new URL(
  '../../../../crates/biorouter-mcp/src/app_folder/resolution-cases.json',
  import.meta.url
);

const temporary: string[] = [];
function tempBase(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'br-app-folder-'));
  temporary.push(dir);
  return dir;
}
afterEach(() => {
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
  return path.join(root, ...rest.filter(Boolean).map((s) => (s === 'marker' ? MOVE_MARKER : s)));
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

const refusingIo: FolderIo = {
  ...NODE_FOLDER_IO,
  rename: () => {
    throw Object.assign(new Error('a file in the folder is held open'), { code: 'EPERM' });
  },
};

describe('the table both implementations run', () => {
  const cases: Case[] = JSON.parse(fs.readFileSync(TABLE, 'utf-8')).cases;

  it('has every case the daemon suite counts on', () => {
    expect(cases.length).toBeGreaterThanOrEqual(9);
  });

  for (const testCase of cases) {
    it(testCase.name, () => {
      const base = tempBase();
      for (const spec of testCase.before) materialize(base, spec);
      const root = appRootUnder(base);
      const io = testCase.renameFails ? refusingIo : NODE_FOLDER_IO;
      expect(testCase.runs.map(() => moveLegacyRoot(root, true, io).token)).toEqual(testCase.runs);
      expect(resolveAppRoot(root)).toBe(tokenPath(base, testCase.resolves));
      for (const spec of testCase.exists) {
        expect(fs.existsSync(tokenPath(base, spec)), `${spec} is missing`).toBe(true);
      }
      for (const spec of testCase.absent) {
        expect(fs.existsSync(tokenPath(base, spec)), `${spec} should not exist`).toBe(false);
      }
    });
  }
});

describe('the move for a whole process', () => {
  it('does nothing off Windows, even with a legacy folder present', () => {
    const base = tempBase();
    materialize(base, 'legacy/config/config.yaml');
    const move = moveForProcess({ platform: 'darwin', env: { APPDATA: base }, homedir: base });
    expect(move).toEqual({ skipped: 'not-windows' });
    expect(fs.existsSync(tokenPath(base, 'legacy/config/config.yaml'))).toBe(true);
  });

  it('does nothing under a sandbox root, and a blank root is no root', () => {
    const base = tempBase();
    materialize(base, 'legacy/config/config.yaml');
    const env = { APPDATA: base, LOCALAPPDATA: base };
    expect(
      moveForProcess({
        platform: 'win32',
        env: { ...env, BIOROUTER_PATH_ROOT: '/sb' },
        homedir: base,
      })
    ).toEqual({ skipped: 'sandboxed' });
    expect(fs.existsSync(tokenPath(base, 'current'))).toBe(false);

    const move = moveForProcess({
      platform: 'win32',
      env: { ...env, BIOROUTER_PATH_ROOT: '   ' },
      homedir: base,
    });
    expect(move.roaming?.outcome.token).toBe('moved');
  });

  it('moves both bases and marks only the roaming one for the daemon to finish', () => {
    const roaming = tempBase();
    const local = tempBase();
    materialize(roaming, 'legacy/config/config.yaml');
    materialize(local, 'legacy/cache/autovisualiser/figure.html');
    const move = moveForProcess({
      platform: 'win32',
      env: { APPDATA: roaming, LOCALAPPDATA: local },
      homedir: tempBase(),
    });
    expect(move.roaming?.outcome.token).toBe('moved');
    expect(move.local?.outcome.token).toBe('moved');
    expect(fs.existsSync(tokenPath(roaming, 'current/marker'))).toBe(true);
    expect(fs.existsSync(tokenPath(local, 'current/marker'))).toBe(false);
    expect(fs.existsSync(tokenPath(local, 'current/cache/autovisualiser/figure.html'))).toBe(true);
  });

  it('falls back to the home folder when APPDATA and LOCALAPPDATA are blank', () => {
    const home = tempBase();
    const roaming = path.join(home, 'AppData', 'Roaming');
    materialize(roaming, 'legacy/config/config.yaml');
    const move = moveForProcess({
      platform: 'win32',
      env: { APPDATA: '', LOCALAPPDATA: '' },
      homedir: home,
    });
    expect(move.roaming?.root.current).toBe(path.join(roaming, APP_AUTHOR, APP_NAME));
    expect(move.roaming?.outcome.token).toBe('moved');
    expect(move.local?.root.current).toBe(
      path.join(home, 'AppData', 'Local', APP_AUTHOR, APP_NAME)
    );
  });

  it('says what it did, naming both folders, and is quiet when there was nothing to do', () => {
    const roaming = tempBase();
    materialize(roaming, 'legacy/config/config.yaml');
    const moved = moveForProcess({
      platform: 'win32',
      env: { APPDATA: roaming, LOCALAPPDATA: tempBase() },
      homedir: roaming,
    });
    const lines = describeFolderMove(moved);
    expect(lines).toHaveLength(1);
    expect(lines[0].level).toBe('info');
    expect(lines[0].text).toContain(tokenPath(roaming, 'current'));
    expect(lines[0].text).toContain(tokenPath(roaming, 'legacy'));

    const failed = tempBase();
    materialize(failed, 'legacy/config/config.yaml');
    const refused = moveForProcess({
      platform: 'win32',
      env: { APPDATA: failed, LOCALAPPDATA: tempBase() },
      homedir: failed,
      io: refusingIo,
    });
    expect(describeFolderMove(refused).map((line) => line.level)).toEqual(['warn']);
    expect(describeFolderMove({ skipped: 'not-windows' })).toEqual([]);
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
  const resolveWith = (present: string[], env: Record<string, string | undefined>) =>
    platformConfigDirFor({
      platform: 'win32',
      env,
      homedir: home,
      pathImpl: path.win32,
      exists: (target) => present.includes(target),
    });

  it('names the current folder on a fresh install', () => {
    expect(resolveWith([], { APPDATA: appData })).toBe(
      'C:\\Users\\Ana\\AppData\\Roaming\\UCSF\\biorouter\\config'
    );
  });

  it('keeps the legacy folder while the move has not happened', () => {
    expect(resolveWith([legacy], { APPDATA: appData })).toBe(path.win32.join(legacy, 'config'));
  });

  it('prefers the current folder when both exist', () => {
    expect(resolveWith([legacy, current], { APPDATA: appData })).toBe(
      path.win32.join(current, 'config')
    );
  });

  it('joins the home folder when APPDATA is blank, as etcetera does', () => {
    expect(resolveWith([], { APPDATA: '' })).toBe(
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
