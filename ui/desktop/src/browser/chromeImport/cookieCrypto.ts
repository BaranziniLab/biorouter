import crypto from 'node:crypto';

/**
 * Chrome's cookie encryption on macOS and Linux (BB-22, "Reading Chrome's files").
 *
 * An encrypted value is a three byte version prefix and AES-128-CBC ciphertext with PKCS#7
 * padding, under a 16 byte key derived with PBKDF2-HMAC-SHA1 (salt `saltysalt`) and an IV of 16
 * spaces:
 *
 * - macOS `v10`: the key comes from the Keychain password, 1,003 iterations.
 * - Linux `v10`: the fixed password `peanuts`, 1 iteration (Chrome without a key store).
 * - Linux `v11`: the Secret Service password, 1 iteration.
 * - `v20`: Windows app-bound encryption, tied to Chrome itself; never decrypted (BB-24).
 *
 * From cookie database version 24, the plaintext starts with the SHA-256 of the cookie's
 * `host_key`. That prefix is checked, not just cut: a value that does not carry its own host's
 * digest is dropped rather than imported as garbage, or as another site's cookie.
 */

export const CHROME_SALT = 'saltysalt';
export const CHROME_KEY_BYTES = 16;
export const MACOS_ITERATIONS = 1003;
export const LINUX_ITERATIONS = 1;
export const LINUX_V10_PASSWORD = 'peanuts';
/** The first cookie database version whose values carry the host digest. */
export const DIGEST_PREFIX_VERSION = 24;
const IV = Buffer.alloc(16, 0x20);
const DIGEST_BYTES = 32;

export type CookieKeyVersion = 'v10' | 'v11';
export type CookieKeys = Partial<Record<CookieKeyVersion, Buffer>>;

export type DecryptFailure =
  | 'no-key'
  | 'unknown-prefix'
  | 'app-bound'
  | 'bad-ciphertext'
  | 'digest-mismatch'
  | 'not-text';

export type DecryptOutcome = { ok: true; value: string } | { ok: false; reason: DecryptFailure };

export function deriveChromeKey(password: string | Buffer, iterations: number): Buffer {
  return crypto.pbkdf2Sync(
    password,
    CHROME_SALT,
    Math.max(1, iterations),
    CHROME_KEY_BYTES,
    'sha1'
  );
}

/** The version prefix of an encrypted value: `v10`, `v11`, `v20`, or `null` for anything else. */
export function cookieValueVersion(encrypted: Uint8Array): 'v10' | 'v11' | 'v20' | null {
  if (encrypted.length < 3) return null;
  const prefix = Buffer.from(encrypted.subarray(0, 3)).toString('latin1');
  return prefix === 'v10' || prefix === 'v11' || prefix === 'v20' ? prefix : null;
}

const UTF8 = new TextDecoder('utf-8', { fatal: true });

export function decryptCookieValue(
  encrypted: Uint8Array,
  keys: CookieKeys,
  options: { hostKey: string; digestPrefixed: boolean }
): DecryptOutcome {
  const version = cookieValueVersion(encrypted);
  if (version === null) return { ok: false, reason: 'unknown-prefix' };
  if (version === 'v20') return { ok: false, reason: 'app-bound' };
  const key = keys[version];
  if (!key) return { ok: false, reason: 'no-key' };

  const ciphertext = Buffer.from(encrypted.subarray(3));
  if (ciphertext.length === 0 || ciphertext.length % 16 !== 0) {
    return { ok: false, reason: 'bad-ciphertext' };
  }
  let plaintext: Buffer;
  try {
    const decipher = crypto.createDecipheriv('aes-128-cbc', key, IV);
    plaintext = Buffer.concat([decipher.update(ciphertext), decipher.final()]);
  } catch {
    return { ok: false, reason: 'bad-ciphertext' };
  }
  try {
    if (options.digestPrefixed) {
      if (plaintext.length < DIGEST_BYTES) return { ok: false, reason: 'digest-mismatch' };
      const expected = crypto.createHash('sha256').update(options.hostKey, 'utf8').digest();
      if (!crypto.timingSafeEqual(plaintext.subarray(0, DIGEST_BYTES), expected)) {
        return { ok: false, reason: 'digest-mismatch' };
      }
      plaintext = plaintext.subarray(DIGEST_BYTES);
    }
    return { ok: true, value: UTF8.decode(plaintext) };
  } catch {
    return { ok: false, reason: 'not-text' };
  } finally {
    plaintext.fill(0);
  }
}

/**
 * Encrypt a value the way Chrome does. Used by the synthetic profile builder and the tests, so the
 * fixtures go through the same PBKDF2 and AES path as a real profile, never copied from one.
 */
export function encryptCookieValue(
  value: string,
  key: Buffer,
  options: { version: CookieKeyVersion; hostKey: string; digestPrefixed: boolean }
): Buffer {
  const body = Buffer.from(value, 'utf8');
  const plaintext = options.digestPrefixed
    ? Buffer.concat([crypto.createHash('sha256').update(options.hostKey, 'utf8').digest(), body])
    : body;
  const cipher = crypto.createCipheriv('aes-128-cbc', key, IV);
  return Buffer.concat([
    Buffer.from(options.version, 'latin1'),
    cipher.update(plaintext),
    cipher.final(),
  ]);
}
