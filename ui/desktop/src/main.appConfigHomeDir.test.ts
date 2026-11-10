import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The renderer writes a path as `~/…` from `window.appConfig.get('BIOROUTER_HOME_DIR')`. The main
 * process once never put the key in the config it hands every window, so every reader got
 * `undefined` and showed `/Users/me/Desktop` and a folder group named `me` instead of `~/Desktop`
 * and `~` (vision V1-M6). The main process cannot be mounted in a unit test, so the config is read
 * at the source, next to the reader, so the key cannot drift apart from it again.
 *
 * ONE renderer file reads the key: `homeFolder()` in utils/folderPath.ts, which also decides when
 * `~` would mislead (a daemon that may be on another machine). The sidebar (its folder groups and
 * hover card), the composer footer's folder chip and its card, History and the terminal dock all
 * take the home folder from it, so a folder is written one way everywhere. A surface that read the
 * key itself would skip that decision, which is how the sidebar once wrote `~` for an external
 * backend while every other surface wrote the path absolute.
 */
const MAIN = readFileSync(join(__dirname, 'main.ts'), 'utf8');
const READER = 'utils/folderPath.ts';
const READER_SOURCE = readFileSync(join(__dirname, READER), 'utf8');

/** Every surface that writes a folder against the home folder, and the import that gets it. */
const SURFACES = [
  ['components/BioRouterSidebar/AppSidebar.tsx', '../../utils/folderPath'],
  ['components/InAppTerminalDock.tsx', '../utils/folderPath'],
  ['components/TerminalCwd.tsx', '../utils/folderPath'],
  ['components/bottom_menu/DirSwitcher.tsx', '../../utils/folderPath'],
  ['components/bottom_menu/FooterCard.tsx', '../../utils/folderPath'],
  ['components/sessions/SessionListView.tsx', '../../utils/folderPath'],
  ['components/sessions/SessionHistoryView.tsx', '../../utils/folderPath'],
].map(([file, from]) => ({ file, from, source: readFileSync(join(__dirname, file), 'utf8') }));

describe('BIOROUTER_HOME_DIR', () => {
  it('is in the config object every window is created with, set to the OS home folder', () => {
    const start = MAIN.indexOf('let appConfig = {');
    expect(start, 'no appConfig object in main.ts').toBeGreaterThan(-1);
    const object = MAIN.slice(start, MAIN.indexOf('\n};\n', start));
    expect(object).toMatch(/^\s*BIOROUTER_HOME_DIR: os\.homedir\(\),$/m);
  });

  it('reaches every window: the per-window config spreads appConfig and does not override it', () => {
    expect(MAIN).toMatch(/JSON\.stringify\(\{\s*\.\.\.appConfig,/);
    expect(MAIN).not.toMatch(/^\s*BIOROUTER_HOME_DIR: (?!os\.homedir\(\),)/m);
  });

  it(`is read by ${READER} under the same name, inside homeFolder()`, () => {
    const start = READER_SOURCE.indexOf('export function homeFolder()');
    expect(start, 'no homeFolder() in folderPath.ts').toBeGreaterThan(-1);
    const body = READER_SOURCE.slice(start, READER_SOURCE.indexOf('\n}\n', start));
    expect(body).toContain("get('BIOROUTER_HOME_DIR')");
  });

  it.each(SURFACES)(
    '$file takes the home folder from homeFolder() and never reads the key itself',
    ({ source, from }) => {
      expect(source).toMatch(
        new RegExp(`import \\{[^}]*\\bhomeFolder\\b[^}]*\\} from '${from.replace(/\./g, '\\.')}';`)
      );
      expect(source).toContain('homeFolder()');
      expect(source).not.toContain('BIOROUTER_HOME_DIR');
    }
  );
});

describe('backend ownership notice', () => {
  it('takes the per-window flag from the actual backend result, including environment-selected external servers', () => {
    expect(MAIN).toContain('BIOROUTER_LOCAL_BACKEND: biorouterdResult.managed,');
    expect(MAIN).not.toContain('BIOROUTER_LOCAL_BACKEND: !settings.externalBiorouterd?.enabled,');
  });
});
