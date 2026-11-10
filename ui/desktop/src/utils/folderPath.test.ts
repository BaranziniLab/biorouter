import { afterEach, describe, expect, it } from 'vitest';
import {
  folderName,
  folderSegments,
  homeFolder,
  normalizeFolder as movedNormalizeFolder,
  tildePath as movedTildePath,
} from './folderPath';
import {
  CREW_FOLDER_LABEL,
  folderLabels,
  normalizeFolder,
  tildePath,
} from '../components/BioRouterSidebar/sidebarChatView';

/**
 * The one rule for writing a folder (utils/folderPath.ts). The sidebar's folder groups, the
 * composer footer's chip and its card all read it, so a folder reads the same in each: `~` for the
 * home folder, absolute elsewhere, and the folder's own name where there is room for only a word.
 */

const HOME = '/Users/me';
const CREW_DIR = '/Users/me/Library/Application Support/Biorouter/crew/tasks';

/** Stand in for the config the main process gives every window (`BIOROUTER_*`). */
function setConfig(config: Record<string, unknown> | undefined) {
  Object.assign(window, {
    appConfig: config === undefined ? undefined : { get: (key: string) => config[key] },
  });
}

afterEach(() => {
  setConfig(undefined);
  delete document.documentElement.dataset.biorouterSurface;
});

describe('normalizeFolder and tildePath moved here', () => {
  it('are the very functions the sidebar module still exports', () => {
    expect(normalizeFolder).toBe(movedNormalizeFolder);
    expect(tildePath).toBe(movedTildePath);
  });

  it('keep their behaviour, drive-root quirk included', () => {
    expect(normalizeFolder('/Users/me/Desktop/')).toBe('/Users/me/Desktop');
    expect(normalizeFolder('/')).toBe('/');
    expect(normalizeFolder('  ')).toBe('');
    expect(normalizeFolder(null)).toBe('');
    // A known quirk, left for a follow-up because the sidebar keys its folder groups on it.
    expect(normalizeFolder('C:\\')).toBe('C:');
    expect(tildePath('/Users/me', HOME)).toBe('~');
    expect(tildePath('/Users/me/Desktop/BioRouter', HOME)).toBe('~/Desktop/BioRouter');
    expect(tildePath('/Users/me/Desktop', `${HOME}/`)).toBe('~/Desktop');
    expect(tildePath('/Users/meta/x', HOME)).toBe('/Users/meta/x');
    expect(tildePath('/Volumes/Lab/data', HOME)).toBe('/Volumes/Lab/data');
    expect(tildePath('C:\\Users\\me\\Desktop', 'C:\\Users\\me')).toBe('~\\Desktop');
    expect(tildePath('/Users/me/x', null)).toBe('/Users/me/x');
  });
});

describe('homeFolder', () => {
  it('is the home folder while the daemon is the desktop’s own', () => {
    setConfig({ BIOROUTER_HOME_DIR: HOME, BIOROUTER_LOCAL_BACKEND: true });
    expect(homeFolder()).toBe(HOME);
  });

  it('is unknown with no config, no home folder, or an empty one', () => {
    setConfig(undefined);
    expect(homeFolder()).toBeNull();
    setConfig({ BIOROUTER_LOCAL_BACKEND: true });
    expect(homeFolder()).toBeNull();
    setConfig({ BIOROUTER_HOME_DIR: '  ', BIOROUTER_LOCAL_BACKEND: true });
    expect(homeFolder()).toBeNull();
  });

  it('is unknown for a daemon that may run on another machine, where `~` names the wrong home', () => {
    // An external backend (BIOROUTER_EXTERNAL_BACKEND): the desktop did not start the daemon.
    setConfig({ BIOROUTER_HOME_DIR: HOME, BIOROUTER_LOCAL_BACKEND: false });
    expect(homeFolder()).toBeNull();
    setConfig({ BIOROUTER_HOME_DIR: HOME });
    expect(homeFolder()).toBeNull();
    // A `biorouter serve` browser, even if a config carried both.
    setConfig({ BIOROUTER_HOME_DIR: HOME, BIOROUTER_LOCAL_BACKEND: true });
    document.documentElement.dataset.biorouterSurface = 'headless';
    expect(homeFolder()).toBeNull();
  });
});

describe('folderSegments', () => {
  it.each([
    ['~/Desktop/BioRouter', ['~', '/', 'Desktop', '/', 'BioRouter']],
    ['/Volumes/Lab/data', ['/', 'Volumes', '/', 'Lab', '/', 'data']],
    ['~', ['~']],
    ['/', ['/']],
    ['C:\\Users\\me', ['C:', '\\', 'Users', '\\', 'me']],
    ['\\\\server\\share', ['\\\\', 'server', '\\', 'share']],
    ['~/Mobile Documents/x', ['~', '/', 'Mobile Documents', '/', 'x']],
    ['/a//b/', ['/', 'a', '//', 'b', '/']],
  ])('splits %s into names and separators', (path, pieces) => {
    const segments = folderSegments(path);
    expect(segments.map((segment) => segment.text)).toEqual(pieces);
    // Every character kept, so a selection of the drawn path copies the path.
    expect(segments.map((segment) => segment.text).join('')).toBe(path);
    for (const segment of segments) {
      expect(segment.separator).toBe(/^[\\/]+$/.test(segment.text));
    }
  });

  it('has nothing to draw for an empty path', () => {
    expect(folderSegments('')).toEqual([]);
  });
});

describe('folderName', () => {
  it.each([
    ['an ordinary folder', '/Users/me/Desktop', 'Desktop'],
    ['a trailing separator', '/Users/me/Downloads/', 'Downloads'],
    ['a Windows folder', 'C:\\Users\\me\\Desktop', 'Desktop'],
    ['the home folder', HOME, '~'],
    ['the home folder with a trailing separator', `${HOME}/`, '~'],
    ['the Crew task folder', CREW_DIR, 'Crew'],
    ['the root', '/', '/'],
    ['a Windows drive root, as written', 'C:\\', 'C:\\'],
    ['a forward-slash drive root', 'C:/', 'C:/'],
    ['a doubled drive-root separator', 'd:\\\\', 'd:\\\\'],
    ['nothing', '', ''],
  ])('names %s', (_case, path, name) => {
    expect(folderName(path, HOME)).toBe(name);
  });

  it('reads the home folder by its own name when the home folder is not known', () => {
    expect(folderName(HOME, null)).toBe('me');
    expect(folderName(HOME)).toBe('me');
  });

  it('gives the word the sidebar’s `folderLabels` gives, for the same home folder (one folder, no collisions)', () => {
    // The two HELPERS, given one home folder. Whether the two SURFACES are given the same home
    // folder is a different question, asked of the rendered sidebar and chip under one
    // `window.appConfig` in BioRouterSidebar/folderNameAgreement.test.tsx.
    // Drive roots are left out on purpose: the sidebar keys its groups on `normalizeFolder`, which
    // turns `C:\` into `C:` (a separate follow-up), while the chip keeps the separator.
    for (const folder of [
      '/Users/me/Desktop',
      '/Users/me/Projects/kdps',
      HOME,
      CREW_DIR,
      '/',
      'C:\\Users\\me\\Desktop',
    ]) {
      expect(folderName(folder, HOME), folder).toBe(folderLabels([folder], HOME).get(folder));
    }
    expect(folderName(CREW_DIR, HOME)).toBe(CREW_FOLDER_LABEL);
  });
});
