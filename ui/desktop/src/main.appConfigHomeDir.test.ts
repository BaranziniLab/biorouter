import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Two renderer files read `window.appConfig.get('BIOROUTER_HOME_DIR')` to write a path as `~/…`:
 * the sidebar (its hover card and folder groups) and the terminal dock. The main process never
 * put the key in the config it hands every window, so both read `undefined` and showed
 * `/Users/me/Desktop` and a folder group named `me` instead of `~/Desktop` and `~` (vision V1-M6).
 * The main process cannot be mounted in a unit test, so the config is read at the source, next to
 * the readers, so the key cannot drift apart from them again.
 */
const MAIN = readFileSync(join(__dirname, 'main.ts'), 'utf8');
const READERS = [
  'components/BioRouterSidebar/AppSidebar.tsx',
  'components/InAppTerminalDock.tsx',
].map((file) => ({ file, source: readFileSync(join(__dirname, file), 'utf8') }));

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

  it.each(READERS)('is read by $file under the same name', ({ source }) => {
    expect(source).toContain("get('BIOROUTER_HOME_DIR')");
  });
});
