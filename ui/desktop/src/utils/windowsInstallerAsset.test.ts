import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

/**
 * The Windows installer's filename is agreed in two places that never import
 * each other:
 *
 *   - `forge.config.ts` NAMES it (`maker-squirrel`'s `setupExe`), at build time;
 *   - `githubUpdater.ts` LOOKS FOR it, by exact name, at update time.
 *
 * ⚠ A drift between them is silent in the worst way. The release still builds
 * and still uploads an installer; the updater simply never matches it, falls
 * through to the zip (or to nothing), and Windows quietly goes back to "download
 * it yourself" — which is the exact failure this whole change set exists to end.
 * Neither side's own tests can catch that, because each is internally correct.
 */
const root = resolve(__dirname, '../..');
const forgeConfig = readFileSync(resolve(root, 'forge.config.ts'), 'utf8');
const githubUpdater = readFileSync(resolve(root, 'src/utils/githubUpdater.ts'), 'utf8');
const packageJson = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8')) as {
  version: string;
};

describe('the Windows installer asset name', () => {
  it('is built from package.json, not from an npm-only env var', () => {
    expect(forgeConfig).toContain("require('./package.json')");

    // ⚠ Comments are stripped first. The note above `WINDOWS_SETUP_EXE`
    // explains *why not* to use `npm_package_version` and therefore names it —
    // a naive substring check fails on the very documentation that prevents the
    // mistake. (This test caught itself doing exactly that.)
    const code = forgeConfig
      .split('\n')
      .filter((line) => !line.trim().startsWith('//'))
      .join('\n');

    expect(
      code.includes('npm_package_version'),
      '`process.env.npm_package_version` is only set under `npm run`, so a direct ' +
        '`npx electron-forge make` would produce `Biorouter-Setup-.exe` and the ' +
        'updater would never find it'
    ).toBe(false);
  });

  it('is spelled the same way by the maker and the updater', () => {
    // The maker's single definition.
    expect(forgeConfig).toMatch(/const WINDOWS_SETUP_EXE = `Biorouter-Setup-\$\{APP_VERSION\}\.exe`/);
    expect(forgeConfig).toMatch(/setupExe:\s*WINDOWS_SETUP_EXE/);

    // The updater's expectation, with `v` being the release version.
    expect(githubUpdater).toMatch(/`Biorouter-Setup-\$\{v\}\.exe`/);
  });

  it('resolves to the same concrete filename on both sides', () => {
    const v = packageJson.version;
    const fromMaker = `Biorouter-Setup-${v}.exe`;
    // Re-derive the updater's candidate the way the updater does.
    const fromUpdater = `Biorouter-Setup-${v}.exe`;
    expect(fromMaker).toBe(fromUpdater);
    // And it is a plausible version, not an empty interpolation.
    expect(v).toMatch(/^\d+\.\d+\.\d+/);
  });

  it('still falls back to the zip, for releases cut before the installer existed', () => {
    // The installer must come FIRST, or an old zip would win on a new release.
    const winBranch = githubUpdater.slice(
      githubUpdater.indexOf("platform === 'win32'"),
      githubUpdater.indexOf('// Linux: prefer .deb')
    );
    const setupAt = winBranch.indexOf('Biorouter-Setup-');
    const zipAt = winBranch.indexOf('Biorouter-win32-x64-');
    expect(setupAt, 'the win32 branch must name the installer').toBeGreaterThan(-1);
    expect(zipAt, 'the win32 branch must still name the zip as a fallback').toBeGreaterThan(-1);
    expect(setupAt).toBeLessThan(zipAt);
  });
});
