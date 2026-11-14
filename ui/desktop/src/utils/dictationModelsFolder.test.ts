// @vitest-environment node

/**
 * The folder Settings' **Show in Finder** may create and open (`dictation:show-models-folder` in
 * `main.ts`, DD-66). Main derives this account's speech model folder itself and reads the
 * renderer's `status.models_dir` only as a hint, so these cases hold the derivation to the
 * daemon's `Paths::in_local_data_dir("dictation/models")` and hold every other hint to `null`.
 * The Windows branch runs on every OS through `path.win32`.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { appRootUnder, type Presence } from './appFolder';
import {
  MAX_FOLDER_HINT,
  admitDictationModelsFolder,
  dictationModelsFolderFor,
  showDictationModelsFolder,
  type DictationFolderInputs,
  type ShowFolderDeps,
} from './dictationModelsFolder';

const HOME = '/home/ana';
const OWN = '/home/ana/.local/share/biorouter/dictation/models';

const posix = (
  env: Record<string, string | undefined> = {},
  platform: typeof process.platform = 'linux'
): DictationFolderInputs => ({ platform, env, homedir: HOME, pathImpl: path.posix });

const WIN_HOME = 'C:\\Users\\ana';
const WIN_LOCAL = 'C:\\Users\\ana\\AppData\\Local';
const winRoot = appRootUnder(WIN_LOCAL, path.win32.join);

const windows = (presence: (target: string) => Presence): DictationFolderInputs => ({
  platform: 'win32',
  env: { APPDATA: 'C:\\Users\\ana\\AppData\\Roaming', LOCALAPPDATA: WIN_LOCAL },
  homedir: WIN_HOME,
  pathImpl: path.win32,
  presence,
});
/** The daemon's current folder is there, as after its move. */
const currentOnly = (target: string): Presence =>
  target === winRoot.current ? 'present' : 'absent';
/** Only the folder earlier releases used is there: the daemon has not moved it yet. */
const legacyOnly = (target: string): Presence => (target === winRoot.legacy ? 'present' : 'absent');

describe('the folder main derives, by the daemon’s rule', () => {
  it('is the XDG data folder on Linux and on macOS', () => {
    expect(dictationModelsFolderFor(posix())).toBe(OWN);
    expect(dictationModelsFolderFor(posix({}, 'darwin'))).toBe(OWN);
  });

  it('follows an absolute XDG_DATA_HOME, also one outside the home folder', () => {
    expect(dictationModelsFolderFor(posix({ XDG_DATA_HOME: '/srv/xdg' }))).toBe(
      '/srv/xdg/biorouter/dictation/models'
    );
  });

  it('ignores a relative XDG_DATA_HOME, as etcetera does', () => {
    expect(dictationModelsFolderFor(posix({ XDG_DATA_HOME: 'share' }))).toBe(OWN);
  });

  it('is <root>/data/dictation/models under BIOROUTER_PATH_ROOT, and a blank root is absent', () => {
    expect(dictationModelsFolderFor(posix({ BIOROUTER_PATH_ROOT: '/tmp/sandbox' }))).toBe(
      '/tmp/sandbox/data/dictation/models'
    );
    expect(
      dictationModelsFolderFor(posix({ BIOROUTER_PATH_ROOT: '/tmp/sandbox', XDG_DATA_HOME: '/x' }))
    ).toBe('/tmp/sandbox/data/dictation/models');
    expect(dictationModelsFolderFor(posix({ BIOROUTER_PATH_ROOT: '   ' }))).toBe(OWN);
  });

  it('names no folder for a relative BIOROUTER_PATH_ROOT', () => {
    expect(dictationModelsFolderFor(posix({ BIOROUTER_PATH_ROOT: 'sandbox' }))).toBeNull();
    expect(
      admitDictationModelsFolder(
        'sandbox/data/dictation/models',
        posix({ BIOROUTER_PATH_ROOT: 'sandbox' })
      )
    ).toBeNull();
  });

  it('is under %LOCALAPPDATA% on Windows, never the roaming folder', () => {
    expect(dictationModelsFolderFor(windows(currentOnly))).toBe(
      path.win32.join(winRoot.current, 'data', 'dictation', 'models')
    );
  });

  it('is the legacy folder’s on Windows while the daemon has not moved it', () => {
    expect(dictationModelsFolderFor(windows(legacyOnly))).toBe(
      path.win32.join(winRoot.legacy, 'data', 'dictation', 'models')
    );
  });
});

describe('the hint the renderer sends', () => {
  it('admits this account’s folder, and answers with the derived path', () => {
    expect(admitDictationModelsFolder(OWN, posix())).toBe(OWN);
    expect(
      admitDictationModelsFolder('/srv/xdg/biorouter/dictation/models', {
        ...posix({ XDG_DATA_HOME: '/srv/xdg' }),
      })
    ).toBe('/srv/xdg/biorouter/dictation/models');
    expect(
      admitDictationModelsFolder(
        '/tmp/sandbox/data/dictation/models',
        posix({ BIOROUTER_PATH_ROOT: '/tmp/sandbox' })
      )
    ).toBe('/tmp/sandbox/data/dictation/models');
  });

  it('admits the Windows folder as the daemon spells it, separators and case aside', () => {
    const folder = path.win32.join(winRoot.current, 'data', 'dictation', 'models');
    // `PathBuf::join("dictation/models")` keeps the forward slash on Windows.
    const daemon = `${path.win32.join(winRoot.current, 'data')}\\dictation/models`;
    expect(admitDictationModelsFolder(daemon, windows(currentOnly))).toBe(folder);
    expect(admitDictationModelsFolder(folder.toUpperCase(), windows(currentOnly))).toBe(folder);
    const roaming = path.win32.join(
      appRootUnder('C:\\Users\\ana\\AppData\\Roaming', path.win32.join).current,
      'data',
      'dictation',
      'models'
    );
    expect(admitDictationModelsFolder(roaming, windows(currentOnly))).toBeNull();
  });

  const refused: [string, unknown, DictationFolderInputs?][] = [
    ['a non string', 42],
    ['undefined', undefined],
    ['null', null],
    ['an object', { toString: () => OWN }],
    ['an empty string', ''],
    ['a path over the length limit', `/${'a'.repeat(MAX_FOLDER_HINT)}/dictation/models`],
    ['a relative path', 'dictation/models'],
    ['a tilde path', '~/.local/share/biorouter/dictation/models'],
    ['a path with a .. segment that resolves to the folder', `${OWN}/../models`],
    ['a path with a .. segment that leaves it', `${OWN}/../../../../../../etc/dictation/models`],
    ['a trailing slash', `${OWN}/`],
    [
      'another folder in the home folder that ends in dictation/models',
      `${HOME}/x/dictation/models`,
    ],
    ['a folder outside the home folder', '/etc/dictation/models'],
    ['the home folder itself', HOME],
    ['the root', '/'],
    ['the parent of the folder', '/home/ana/.local/share/biorouter/dictation'],
    ['a folder inside it', `${OWN}/whisper-small.en-q8_0`],
    ['a file inside it', `${OWN}/ggml-small.en-q8_0.bin`],
    ['another case, on a case sensitive system', OWN.toUpperCase()],
    [
      'the home folder’s copy, under a sandbox root',
      OWN,
      posix({ BIOROUTER_PATH_ROOT: '/tmp/sb' }),
    ],
    ['the default folder, when XDG_DATA_HOME moves it', OWN, posix({ XDG_DATA_HOME: '/srv/xdg' })],
  ];

  it.each(refused)('refuses %s', (_name, hint, inputs) => {
    expect(admitDictationModelsFolder(hint, inputs ?? posix())).toBeNull();
  });
});

describe('opening it', () => {
  function fakes(real = (target: string) => target): ShowFolderDeps & {
    calls: string[];
  } {
    const calls: string[] = [];
    return {
      calls,
      mkdir: vi.fn(async (target: string) => calls.push(`mkdir ${target}`)),
      realpath: vi.fn(async (target: string) => real(target)),
      isPlainDirectory: vi.fn(async (target: string) => path.posix.extname(target) === ''),
      openPath: vi.fn(async (target: string) => {
        calls.push(`open ${target}`);
        return '';
      }),
    };
  }

  it('creates the folder when missing and opens its real path', async () => {
    const deps = fakes();
    expect(await showDictationModelsFolder(OWN, posix(), deps)).toBe(true);
    expect(deps.calls).toEqual([`mkdir ${OWN}`, `open ${OWN}`]);
  });

  it('creates nothing and opens nothing for a hint that names another folder', async () => {
    const deps = fakes();
    expect(await showDictationModelsFolder(`${HOME}/x/dictation/models`, posix(), deps)).toBe(
      false
    );
    expect(await showDictationModelsFolder('/etc/dictation/models', posix(), deps)).toBe(false);
    expect(deps.mkdir).not.toHaveBeenCalled();
    expect(deps.openPath).not.toHaveBeenCalled();
  });

  it('opens nothing when the real path is a bundle the file manager would launch', async () => {
    const deps = fakes(() => '/Applications/Anything.app');
    expect(await showDictationModelsFolder(OWN, posix(), deps)).toBe(false);
    expect(deps.isPlainDirectory).toHaveBeenCalledWith('/Applications/Anything.app');
    expect(deps.openPath).not.toHaveBeenCalled();
  });

  it('answers false when the shell could not open it', async () => {
    const deps = fakes();
    deps.openPath = vi.fn(async () => 'Failed to open path');
    expect(await showDictationModelsFolder(OWN, posix(), deps)).toBe(false);
  });

  describe('on a real disk', () => {
    let root: string | null = null;
    afterEach(() => {
      if (root) fs.rmSync(root, { recursive: true, force: true });
      root = null;
    });

    it('creates the sandbox’s folder and opens it, and only it', async () => {
      root = fs.mkdtempSync(path.join(os.tmpdir(), 'br-dictation-folder-'));
      const inputs: DictationFolderInputs = {
        platform: process.platform,
        env: { BIOROUTER_PATH_ROOT: root },
        homedir: os.homedir(),
      };
      const folder = path.join(root, 'data', 'dictation', 'models');
      const opened: string[] = [];
      const deps: ShowFolderDeps = {
        mkdir: (target) => fs.promises.mkdir(target, { recursive: true }),
        realpath: (target) => fs.promises.realpath(target),
        isPlainDirectory: async (target) =>
          (await fs.promises.stat(target)).isDirectory() && path.extname(target) === '',
        openPath: async (target) => {
          opened.push(target);
          return '';
        },
      };
      const elsewhere = path.join(root, 'elsewhere', 'dictation', 'models');
      expect(await showDictationModelsFolder(elsewhere, inputs, deps)).toBe(false);
      expect(fs.existsSync(path.join(root, 'elsewhere'))).toBe(false);
      expect(await showDictationModelsFolder(folder, inputs, deps)).toBe(true);
      expect(fs.statSync(folder).isDirectory()).toBe(true);
      expect(opened).toEqual([fs.realpathSync(folder)]);
    });
  });
});
