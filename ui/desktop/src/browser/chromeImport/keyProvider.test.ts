// @vitest-environment node
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { deriveChromeKey, LINUX_ITERATIONS, LINUX_V10_PASSWORD } from './cookieCrypto';
import { runKeyHelper, systemKeyProvider, type HelperResult } from './keyProvider';

let dir: string;
beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'biorouter-key-helper-'));
});
afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));

/** A stand-in for `biorouterd browser-key`: never the real helper, never a real key store. */
function fakeHelper(body: string): string {
  const file = path.join(dir, 'fake-biorouterd');
  fs.writeFileSync(file, `#!/bin/sh\n${body}\n`, { mode: 0o700 });
  return file;
}

describe.skipIf(process.platform === 'win32')('running the key helper (BB-23)', () => {
  it('reads one line of hex and asks for the chrome source', async () => {
    const helper = fakeHelper(
      '[ "$1 $2 $3" = "browser-key --source chrome" ] || exit 9\necho 00112233445566778899aabbccddeeff'
    );
    const result = await runKeyHelper(helper);
    expect(result.ok && result.key.toString('hex')).toBe('00112233445566778899aabbccddeeff');
  });

  it('maps each refusal to its reason', async () => {
    const cases: Array<[number, string]> = [
      [3, 'key-store-off'],
      [4, 'not-found'],
      [5, 'not-allowed'],
      [6, 'unsupported'],
      [1, 'failed'],
    ];
    for (const [code, reason] of cases) {
      const result = await runKeyHelper(fakeHelper(`echo "a sentence" >&2\nexit ${code}`));
      expect(result).toEqual({ ok: false, reason });
    }
  });

  it('refuses output that is not exactly a key', async () => {
    for (const body of [
      'echo not-hex',
      'echo 0011',
      'printf ""',
      'echo 00112233445566778899aabbccddeeff00',
    ]) {
      expect(await runKeyHelper(fakeHelper(body))).toEqual({ ok: false, reason: 'failed' });
    }
  });

  it('gives up on a helper that never answers, and on one that cannot start', async () => {
    expect(await runKeyHelper(fakeHelper('sleep 5'), { timeoutMs: 200 })).toEqual({
      ok: false,
      reason: 'failed',
    });
    expect(await runKeyHelper(path.join(dir, 'missing'))).toEqual({ ok: false, reason: 'failed' });
  });
});

describe('the system key provider', () => {
  const key = Buffer.alloc(16, 7);
  const helper = () => vi.fn(async (): Promise<HelperResult> => ({ ok: true, key }));

  it('on macOS asks the helper for v10 only when v10 cookies are chosen', async () => {
    const ask = helper();
    const provider = systemKeyProvider('darwin', ask);
    expect(await provider(new Set())).toEqual({ ok: true, keys: {} });
    expect(ask).not.toHaveBeenCalled();
    expect(await provider(new Set(['v10']))).toEqual({ ok: true, keys: { v10: key } });
    expect(ask).toHaveBeenCalledTimes(1);
  });

  it('on Linux derives v10 itself and asks the helper only for v11', async () => {
    const ask = helper();
    const provider = systemKeyProvider('linux', ask);
    const v10Only = await provider(new Set(['v10']));
    expect(ask).not.toHaveBeenCalled();
    expect(
      v10Only.ok && v10Only.keys.v10?.equals(deriveChromeKey(LINUX_V10_PASSWORD, LINUX_ITERATIONS))
    ).toBe(true);
    const both = await provider(new Set(['v10', 'v11']));
    expect(ask).toHaveBeenCalledTimes(1);
    expect(both.ok && both.keys.v11).toBe(key);
    // A refused v11 key still leaves the v10 cookies importable.
    const refused = systemKeyProvider('linux', async () => ({ ok: false, reason: 'not-allowed' }));
    const partial = await refused(new Set(['v10', 'v11']));
    expect(partial.ok && Object.keys(partial.keys)).toEqual(['v10']);
    expect(await refused(new Set(['v11']))).toEqual({ ok: false, reason: 'not-allowed' });
  });

  it('on Windows never asks', async () => {
    const ask = helper();
    expect(await systemKeyProvider('win32', ask)(new Set(['v10']))).toEqual({
      ok: false,
      reason: 'unsupported',
    });
    expect(ask).not.toHaveBeenCalled();
  });
});
