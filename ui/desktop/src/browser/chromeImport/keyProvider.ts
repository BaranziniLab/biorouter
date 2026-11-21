import { spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import {
  deriveChromeKey,
  LINUX_ITERATIONS,
  LINUX_V10_PASSWORD,
  MACOS_ITERATIONS,
  type CookieKeys,
  type CookieKeyVersion,
} from './cookieCrypto';
import type { Platform } from './profiles';

/**
 * Where the importer gets the keys for Chrome's encrypted cookies (BB-23, BB-24).
 *
 * The importer asks only after the person chose **Import** with logins selected, and only for the
 * versions the chosen sites' cookies use. On macOS the key comes from the Keychain through
 * `biorouterd browser-key`, behind the operating system's own prompt. On Linux `v10` uses Chrome's
 * fixed password and needs no prompt; `v11` comes from the Secret Service through the same helper.
 * Windows logins are not importable (app-bound encryption).
 *
 * Tests inject {@link fixtureKeyProvider}; nothing in a test can reach the helper, which refuses
 * to run while `BIOROUTER_DISABLE_KEYRING` is set anyway.
 */

export type KeyFailure =
  | 'not-allowed'
  | 'not-found'
  | 'key-store-off'
  | 'unsupported'
  | 'not-from-app'
  | 'failed';

/**
 * `missing`: the keys came back, but one version's key could not be read (Linux `v11`, while
 * `v10` derives from Chrome's fixed password), so its cookies will be skipped; the summary says why.
 */
export type KeyResult =
  | { ok: true; keys: CookieKeys; missing?: KeyFailure }
  | { ok: false; reason: KeyFailure };

export type KeyProvider = (needed: ReadonlySet<CookieKeyVersion>) => Promise<KeyResult>;

/** What a helper run produced. The key never leaves this module as text. */
export type HelperResult = { ok: true; key: Buffer } | { ok: false; reason: KeyFailure };

const EXIT_REASONS: Record<number, KeyFailure> = {
  3: 'key-store-off',
  4: 'not-found',
  5: 'not-allowed',
  6: 'unsupported',
  7: 'not-from-app',
};

/** The one-time mask's length, the key's: 16 bytes (`REQUEST_LEN` in `browser_key.rs`). */
const MASK_BYTES = 16;

/** The person may take a while with the operating system's prompt. */
export const HELPER_TIMEOUT_MS = 5 * 60 * 1000;

/**
 * Run `biorouterd browser-key --source chrome` and read the one line of hex it writes.
 *
 * The helper answers only this process (it checks its parent, `browser_key.rs`), and only with a
 * one-time request: a fresh random mask written on its standard input. It answers the key XOR
 * that mask, so the key itself never crosses the pipe, and this function unmasks it. Standard
 * error is drained and dropped: it holds a sentence at most, never a value, and the importer
 * reports the reason from the exit code.
 */
export function runKeyHelper(
  binary: string,
  options: { timeoutMs?: number; env?: typeof process.env } = {}
): Promise<HelperResult> {
  return new Promise((resolve) => {
    let settled = false;
    const finish = (result: HelperResult) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve(result);
    };
    const mask = randomBytes(MASK_BYTES);
    let child: ReturnType<typeof spawn>;
    try {
      child = spawn(binary, ['browser-key', '--source', 'chrome'], {
        stdio: ['pipe', 'pipe', 'pipe'],
        env: options.env ?? process.env,
        windowsHide: true,
      });
    } catch {
      mask.fill(0);
      resolve({ ok: false, reason: 'failed' });
      return;
    }
    // A helper that exits before reading its request closes the pipe; that is its refusal,
    // reported by its exit code, not an error of this process.
    child.stdin?.on('error', () => {});
    child.stdin?.end(Buffer.from(mask));
    const chunks: Buffer[] = [];
    let size = 0;
    child.stdout?.on('data', (chunk: Buffer) => {
      size += chunk.length;
      if (size <= 256) chunks.push(chunk);
    });
    child.stderr?.on('data', () => {});
    const timer = setTimeout(() => {
      child.kill();
      finish({ ok: false, reason: 'failed' });
    }, options.timeoutMs ?? HELPER_TIMEOUT_MS);
    child.on('error', () => finish({ ok: false, reason: 'failed' }));
    child.on('close', (code) => {
      const output = Buffer.concat(chunks);
      try {
        if (code === 0) {
          const line = output.toString('latin1').trim();
          if (/^[0-9a-f]{32}$/.test(line)) {
            const key = Buffer.from(line, 'hex');
            for (let index = 0; index < key.length; index++) key[index] ^= mask[index];
            finish({ ok: true, key });
            return;
          }
          finish({ ok: false, reason: 'failed' });
          return;
        }
        finish({ ok: false, reason: EXIT_REASONS[code ?? -1] ?? 'failed' });
      } finally {
        output.fill(0);
        mask.fill(0);
        for (const chunk of chunks) chunk.fill(0);
      }
    });
  });
}

/** The keys Chrome on `platform` would use, from the helper where one is needed. */
export function systemKeyProvider(
  platform: Platform,
  helper: () => Promise<HelperResult>
): KeyProvider {
  return async (needed) => {
    const keys: CookieKeys = {};
    if (platform === 'darwin') {
      if (!needed.has('v10')) return { ok: true, keys };
      const result = await helper();
      if (!result.ok) return result;
      keys.v10 = result.key;
      return { ok: true, keys };
    }
    if (platform === 'linux') {
      if (needed.has('v10')) keys.v10 = deriveChromeKey(LINUX_V10_PASSWORD, LINUX_ITERATIONS);
      if (needed.has('v11')) {
        const result = await helper();
        if (!result.ok) {
          return needed.has('v10') ? { ok: true, keys, missing: result.reason } : result;
        }
        keys.v11 = result.key;
      }
      return { ok: true, keys };
    }
    return { ok: false, reason: 'unsupported' };
  };
}

/**
 * A key provider for synthetic profiles: the keys Chrome on `platform` derives from `password`.
 * Test only; main uses it only behind the Electron test hooks (unpackaged and BIOROUTER_E2E=1).
 */
export function fixtureKeyProvider(password: string, platform: 'darwin' | 'linux'): KeyProvider {
  return async (needed) => {
    const keys: CookieKeys = {};
    if (platform === 'darwin') {
      if (needed.has('v10')) keys.v10 = deriveChromeKey(password, MACOS_ITERATIONS);
    } else {
      if (needed.has('v10')) keys.v10 = deriveChromeKey(LINUX_V10_PASSWORD, LINUX_ITERATIONS);
      if (needed.has('v11')) keys.v11 = deriveChromeKey(password, LINUX_ITERATIONS);
    }
    return { ok: true, keys };
  };
}
