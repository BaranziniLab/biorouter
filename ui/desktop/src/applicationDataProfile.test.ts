import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const mocks = vi.hoisted(() => {
  const paths: Record<string, string> = {};
  return {
    paths,
    app: {
      isPackaged: true,
      setPath: vi.fn((key: string, value: string) => {
        paths[key] = value;
      }),
      getPath: vi.fn((key: string) => paths[key]),
      commandLine: {
        appendSwitch: vi.fn(),
        hasSwitch: vi.fn(() => true),
        getSwitchValue: vi.fn(() => '/internal-default-profile'),
        appendArgument: vi.fn(),
        removeSwitch: vi.fn(),
      },
    },
    file: { resolvePathFn: undefined as (() => string) | undefined },
  };
});

vi.mock('electron', () => ({ app: mocks.app }));
vi.mock('electron-log', () => ({
  default: { transports: { file: mocks.file, console: {} } },
}));

import { applyUserDataDirectory } from './applicationDataProfile';

let fixture: string;

beforeEach(() => {
  fixture = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'biorouter-profile-test-')));
  fs.chmodSync(fixture, 0o700);
  vi.clearAllMocks();
  Object.keys(mocks.paths).forEach((key) => delete mocks.paths[key]);
  mocks.paths.userData = '/unchanged-default-profile';
  mocks.paths.sessionData = '/unchanged-default-session';
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
  fs.rmSync(fixture, { recursive: true, force: true });
});

describe('explicit application data profile', () => {
  it('makes no application or filesystem calls without the flag', () => {
    const lstat = vi.spyOn(fs, 'lstatSync');
    const realpath = vi.spyOn(fs, 'realpathSync');
    const mkdir = vi.spyOn(fs, 'mkdirSync');
    expect(applyUserDataDirectory(mocks.app, ['Biorouter', '--other-switch'])).toBeUndefined();
    expect(lstat).not.toHaveBeenCalled();
    expect(realpath).not.toHaveBeenCalled();
    expect(mkdir).not.toHaveBeenCalled();
    expect(mocks.app.setPath).not.toHaveBeenCalled();
    expect(mocks.app.getPath).not.toHaveBeenCalled();
    expect(mocks.app.commandLine.appendSwitch).not.toHaveBeenCalled();
    expect(mocks.app.commandLine.hasSwitch).not.toHaveBeenCalled();
    expect(mocks.app.commandLine.getSwitchValue).not.toHaveBeenCalled();
    expect(mocks.paths.userData).toBe('/unchanged-default-profile');
    expect(mocks.paths.sessionData).toBe('/unchanged-default-session');
  });

  it.each(['equals', 'separate'])('isolates packaged data paths with the %s flag', (form) => {
    const argv = form === 'equals' ? [`--user-data-dir=${fixture}`] : ['--user-data-dir', fixture];
    expect(applyUserDataDirectory(mocks.app, argv)).toBe(fixture);
    expect(mocks.paths).toEqual({
      userData: fixture,
      sessionData: path.join(fixture, 'session'),
      logs: path.join(fixture, 'logs'),
      temp: path.join(fixture, 'temp'),
      crashDumps: path.join(fixture, 'crash-dumps'),
      home: path.join(fixture, 'home'),
      appData: path.join(fixture, 'app-data'),
    });
    expect(mocks.app.commandLine.appendSwitch).toHaveBeenCalledWith('user-data-dir', fixture);
    for (const location of Object.values(mocks.paths)) {
      expect(fs.lstatSync(location).isDirectory()).toBe(true);
      if (process.platform !== 'win32') {
        expect(fs.lstatSync(location).mode & 0o7777).toBe(0o700);
      }
    }
    expect(mocks.app.getPath).not.toHaveBeenCalled();
  });

  it('resolves the actual logger and persistent session partition inside the profile', async () => {
    applyUserDataDirectory(mocks.app, [`--user-data-dir=${fixture}`]);
    await import('./utils/logger');
    expect(mocks.file.resolvePathFn?.()).toBe(path.join(fixture, 'logs', 'main.log'));
    expect(path.join(mocks.app.getPath('sessionData'), 'Partitions', 'crew')).toBe(
      path.join(fixture, 'session', 'Partitions', 'crew')
    );
  });

  it.each([
    { values: [] },
    { values: [''] },
    { values: ['relative-profile'] },
    { values: ['--another-switch'] },
  ])('rejects an invalid separate value $values', ({ values }) => {
    expect(() => applyUserDataDirectory(mocks.app, ['--user-data-dir', ...values])).toThrow();
    expect(mocks.app.setPath).not.toHaveBeenCalled();
  });

  it('rejects duplicate overrides before creating directories or setting paths', () => {
    expect(() =>
      applyUserDataDirectory(mocks.app, [
        `--user-data-dir=${fixture}`,
        `--user-data-dir=${fixture}`,
      ])
    ).toThrow('one absolute');
    expect(fs.readdirSync(fixture)).toEqual([]);
    expect(mocks.app.setPath).not.toHaveBeenCalled();
  });

  it.each(['missing', 'file', 'symlink'])('rejects a %s root', (kind) => {
    const location = path.join(fixture, 'requested');
    if (kind === 'file') fs.writeFileSync(location, 'not a directory');
    if (kind === 'symlink') fs.symlinkSync(fixture, location, 'junction');
    expect(() => applyUserDataDirectory(mocks.app, [`--user-data-dir=${location}`])).toThrow();
    expect(mocks.app.setPath).not.toHaveBeenCalled();
    if (kind === 'missing') expect(fs.existsSync(location)).toBe(false);
  });

  it.each(['file', 'symlink'])('rejects a %s child without writing outside the root', (kind) => {
    const location = path.join(fixture, 'session');
    const target = path.join(fixture, 'outside');
    fs.mkdirSync(target, { mode: 0o700 });
    if (kind === 'file') fs.writeFileSync(location, 'not a directory');
    else fs.symlinkSync(target, location, 'junction');
    expect(() => applyUserDataDirectory(mocks.app, [`--user-data-dir=${fixture}`])).toThrow();
    expect(fs.readdirSync(target)).toEqual([]);
    expect(mocks.app.setPath).not.toHaveBeenCalled();
  });

  it.skipIf(process.platform === 'win32').each([0o755, 0o750, 0o1700, 0o4700])(
    'rejects non-private root mode %o',
    (mode) => {
      fs.chmodSync(fixture, mode);
      expect(() => applyUserDataDirectory(mocks.app, [`--user-data-dir=${fixture}`])).toThrow(
        'mode 0700'
      );
      expect(mocks.app.setPath).not.toHaveBeenCalled();
    }
  );

  it.skipIf(process.platform === 'win32')('rejects a directory owned by another user', () => {
    const actual = fs.lstatSync(fixture);
    vi.spyOn(fs, 'lstatSync').mockReturnValue({
      ...actual,
      isDirectory: () => true,
      isSymbolicLink: () => false,
      uid: process.getuid!() + 1,
    } as fs.Stats);
    expect(() => applyUserDataDirectory(mocks.app, [`--user-data-dir=${fixture}`])).toThrow(
      'owned by this user'
    );
    expect(mocks.app.setPath).not.toHaveBeenCalled();
  });

  it.skipIf(process.platform === 'win32')('rejects a permissive existing session directory', () => {
    fs.mkdirSync(path.join(fixture, 'session'), { mode: 0o755 });
    expect(() => applyUserDataDirectory(mocks.app, [`--user-data-dir=${fixture}`])).toThrow(
      'mode 0700'
    );
    expect(mocks.app.setPath).not.toHaveBeenCalled();
  });

  it('runs before every other main-process import and skips global registration for an override', () => {
    const main = fs.readFileSync(path.join(__dirname, 'main.ts'), 'utf8');
    expect(
      main.startsWith("import { applicationUserDataDirectory } from './applicationDataProfile';")
    ).toBe(true);
    expect(main).toContain(
      'process.env.BIOROUTER_DEV_PROFILE_ROOT || applicationUserDataDirectory'
    );
  });

  it('still rejects development profiles in installed builds before reading default paths', async () => {
    vi.stubEnv('BIOROUTER_DEV_PROFILE_ROOT', fixture);
    vi.resetModules();
    await expect(import('./developmentProfile')).rejects.toThrow(
      'Development profiles are unavailable in installed builds.'
    );
    expect(mocks.app.getPath).not.toHaveBeenCalled();
    expect(mocks.app.setPath).not.toHaveBeenCalled();
  });
});
