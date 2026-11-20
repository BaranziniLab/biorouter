import path from 'node:path';
import { browserTestHooksEnabled } from '../testHooks';
import {
  fixtureKeyProvider,
  runKeyHelper,
  systemKeyProvider,
  type KeyProvider,
} from './keyProvider';
import { chromeDataRoot, type Platform } from './profiles';

/**
 * Where the app's Chrome import reads from, and how it gets the key (BB-21, BB-23).
 *
 * In the app: Chrome's own data folder for this system, and the key through
 * `biorouterd browser-key` behind the operating system's prompt.
 *
 * ⚠ Under the Electron test hooks (an unpackaged app with `BIOROUTER_E2E=1`, the gate the browser's
 * other test hooks use) the import reads ONLY `BIOROUTER_BROWSER_TEST_CHROME_DIR`, a synthetic
 * profile, and derives keys from `BIOROUTER_BROWSER_TEST_CHROME_PASSWORD`, the fixture's password.
 * With the variable unset it finds no Chrome at all: a test or a dev run with the hooks on can never
 * read the person's real profile or reach the real Keychain. A packaged app never reads either
 * variable, whatever its environment holds.
 */

export type ChromeImportSource = {
  platform: Platform;
  dataRoot: string | null;
  keyProvider: KeyProvider;
  /** `fixture` only under the test hooks. */
  kind: 'system' | 'fixture';
};

export function chromeImportSource(options: {
  env: Record<string, string | undefined>;
  isPackaged: boolean;
  platform: Platform;
  home: string;
  /** The signed daemon binary that holds the key helper; resolved only when a key is needed. */
  helperBinary: () => string;
}): ChromeImportSource {
  const { env, platform } = options;
  if (browserTestHooksEnabled(env, options.isPackaged)) {
    const dir = env.BIOROUTER_BROWSER_TEST_CHROME_DIR;
    const testPlatform =
      env.BIOROUTER_BROWSER_TEST_CHROME_PLATFORM === 'linux' ? 'linux' : 'darwin';
    const password = env.BIOROUTER_BROWSER_TEST_CHROME_PASSWORD;
    return {
      platform: platform === 'win32' ? 'win32' : testPlatform,
      dataRoot: dir && path.isAbsolute(dir) ? dir : null,
      keyProvider: password
        ? fixtureKeyProvider(password, testPlatform)
        : async () => ({ ok: false, reason: 'key-store-off' }),
      kind: 'fixture',
    };
  }
  return {
    platform,
    dataRoot: chromeDataRoot(platform, options.home, env),
    keyProvider: systemKeyProvider(platform, () => {
      let binary: string;
      try {
        binary = options.helperBinary();
      } catch {
        return Promise.resolve({ ok: false, reason: 'failed' });
      }
      return runKeyHelper(binary);
    }),
    kind: 'system',
  };
}
