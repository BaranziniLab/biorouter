import fs from 'node:fs';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  LAPTOP_DICTATION_POLICY_KEYS,
  managedPolicyPath,
  readLaptopDictationPolicy,
} from './managedDictationPolicy';

vi.mock('node:fs', () => ({ default: { readFileSync: vi.fn() } }));
const read = vi.mocked(fs.readFileSync);

afterEach(() => vi.resetAllMocks());

describe('laptop dictation policy', () => {
  it('mirrors the managed path for each platform and a blank path root', () => {
    expect(managedPolicyPath({ BIOROUTER_PATH_ROOT: '/tmp/profile' }, 'darwin')).toBe(
      '/tmp/profile/managed/managed-policy.yaml'
    );
    expect(managedPolicyPath({ BIOROUTER_PATH_ROOT: '   ' }, 'darwin')).toBe(
      '/Library/Application Support/Biorouter/managed-policy.yaml'
    );
    expect(managedPolicyPath({}, 'linux')).toBe('/etc/biorouter/managed-policy.yaml');
    expect(managedPolicyPath({ ProgramData: 'C:\\ProgramData' }, 'win32')).toContain('Biorouter');
    expect(managedPolicyPath({}, 'win32')).toBeNull();
  });

  it('allows a missing file and an unrelated policy block', () => {
    read.mockImplementationOnce(() => {
      throw Object.assign(new Error('missing'), { code: 'ENOENT' });
    });
    expect(readLaptopDictationPolicy({}, 'darwin')).toEqual({ enabled: true, capture: true });
    read.mockReturnValueOnce('permissions:\n  deny: []\n');
    expect(readLaptopDictationPolicy({}, 'darwin')).toEqual({ enabled: true, capture: true });
  });

  it('narrows live capture for either false key', () => {
    read.mockReturnValueOnce('dictation:\n  enabled: false\n');
    expect(readLaptopDictationPolicy({}, 'darwin')).toEqual({ enabled: false, capture: false });
    read.mockReturnValueOnce('dictation:\n  capture: false\n');
    expect(readLaptopDictationPolicy({}, 'darwin')).toEqual({ enabled: true, capture: false });
  });

  it('accepts the history key without narrowing live capture', () => {
    read.mockReturnValueOnce('dictation:\n  history: false\n');
    expect(readLaptopDictationPolicy({}, 'darwin')).toEqual({ enabled: true, capture: true });
    read.mockReturnValueOnce('dictation:\n  history: true\n  capture: false\n');
    expect(readLaptopDictationPolicy({}, 'darwin')).toEqual({ enabled: true, capture: false });
  });

  it('knows exactly the keys the daemon knows', async () => {
    // `node:fs` is mocked above; `node:fs/promises` is not, and reads the Rust source.
    const { readFile } = await import('node:fs/promises');
    const source = await readFile(
      path.resolve(__dirname, '../../../../crates/biorouter/src/managed/dictation.rs'),
      'utf8'
    );
    const list = /pub const DICTATION_POLICY_KEYS: \[&str; (\d+)\] = \[([^\]]*)\]/.exec(source);
    expect(list, 'DICTATION_POLICY_KEYS is in managed/dictation.rs').not.toBeNull();
    const rustKeys = [...(list?.[2] ?? '').matchAll(/"([a-z_]+)"/g)].map((m) => m[1]);
    expect(rustKeys).toHaveLength(Number(list?.[1]));
    expect([...LAPTOP_DICTATION_POLICY_KEYS]).toEqual(rustKeys);
    // Each key the daemon knows is read as known here, not as denying.
    for (const key of rustKeys) {
      read.mockReturnValueOnce(`dictation:\n  ${key}: true\n`);
      expect(readLaptopDictationPolicy({}, 'darwin'), key).toEqual({
        enabled: true,
        capture: true,
      });
    }
  });

  it('fails closed on malformed YAML, wrong types and unreadable files', () => {
    read.mockReturnValueOnce('dictation: [');
    expect(readLaptopDictationPolicy({}, 'darwin').capture).toBe(false);
    read.mockReturnValueOnce('dictation:\n  capture: no\n');
    expect(readLaptopDictationPolicy({}, 'darwin').capture).toBe(false);
    read.mockReturnValueOnce('dictation:\n  future_key: true\n');
    expect(readLaptopDictationPolicy({}, 'darwin').capture).toBe(false);
    read.mockImplementationOnce(() => {
      throw new Error('denied');
    });
    expect(readLaptopDictationPolicy({}, 'darwin').capture).toBe(false);
  });
});
