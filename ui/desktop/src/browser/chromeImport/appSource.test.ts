// @vitest-environment node
import os from 'node:os';
import path from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import { deriveChromeKey, MACOS_ITERATIONS } from './cookieCrypto';
import { chromeImportSource } from './appSource';

const base = {
  platform: 'darwin' as const,
  home: '/Users/person',
  helperBinary: () => '/nonexistent/biorouterd',
};

describe('where the app imports from (BB-21, BB-23)', () => {
  it('a packaged app reads Chrome and asks the helper, whatever the environment says', async () => {
    const helperBinary = vi.fn(() => '/nonexistent/biorouterd');
    const source = chromeImportSource({
      ...base,
      helperBinary,
      isPackaged: true,
      env: {
        BIOROUTER_E2E: '1',
        BIOROUTER_BROWSER_TEST_CHROME_DIR: '/tmp/fixture',
        BIOROUTER_BROWSER_TEST_CHROME_PASSWORD: 'fixture',
      },
    });
    expect(source.kind).toBe('system');
    expect(source.dataRoot).toBe('/Users/person/Library/Application Support/Google/Chrome');
    // The helper binary is resolved only when a key is needed, never at start.
    expect(helperBinary).not.toHaveBeenCalled();
  });

  it('an unpackaged app without BIOROUTER_E2E is the app', () => {
    const source = chromeImportSource({
      ...base,
      isPackaged: false,
      env: { BIOROUTER_BROWSER_TEST_CHROME_DIR: '/tmp/fixture' },
    });
    expect(source.kind).toBe('system');
    expect(source.dataRoot).toBe('/Users/person/Library/Application Support/Google/Chrome');
  });

  it('under the test hooks reads only the fixture, and with none finds no Chrome at all', async () => {
    const fixtureDir = path.join(os.tmpdir(), 'fixture-chrome');
    const source = chromeImportSource({
      ...base,
      isPackaged: false,
      env: {
        BIOROUTER_E2E: '1',
        BIOROUTER_BROWSER_TEST_CHROME_DIR: fixtureDir,
        BIOROUTER_BROWSER_TEST_CHROME_PASSWORD: 'fixture-password',
      },
    });
    expect(source).toMatchObject({ kind: 'fixture', dataRoot: fixtureDir, platform: 'darwin' });
    const keys = await source.keyProvider(new Set(['v10']));
    expect(
      keys.ok && keys.keys.v10?.equals(deriveChromeKey('fixture-password', MACOS_ITERATIONS))
    ).toBe(true);

    const bare = chromeImportSource({ ...base, isPackaged: false, env: { BIOROUTER_E2E: '1' } });
    expect(bare.dataRoot).toBeNull();
    // No password: no key, and certainly no Keychain.
    expect(await bare.keyProvider(new Set(['v10']))).toEqual({
      ok: false,
      reason: 'key-store-off',
    });
    const relative = chromeImportSource({
      ...base,
      isPackaged: false,
      env: { BIOROUTER_E2E: '1', BIOROUTER_BROWSER_TEST_CHROME_DIR: 'relative/dir' },
    });
    expect(relative.dataRoot).toBeNull();
  });

  it('a helper that cannot be found is a failed key, not a crash', async () => {
    const source = chromeImportSource({
      ...base,
      isPackaged: true,
      env: {},
      helperBinary: () => {
        throw new Error('no biorouterd');
      },
    });
    expect(await source.keyProvider(new Set(['v10']))).toEqual({ ok: false, reason: 'failed' });
  });
});
