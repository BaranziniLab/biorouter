import { afterEach, describe, expect, it } from 'vitest';
import { isLocalAbsolutePath, localPlatform, type LocalPlatform } from './localPath';

function setPlatform(platform: unknown) {
  Object.defineProperty(window, 'electron', {
    value: platform === undefined ? undefined : { platform },
    configurable: true,
    writable: true,
  });
}

afterEach(() => {
  Reflect.deleteProperty(window, 'electron');
});

describe('localPlatform', () => {
  it('reads the preload’s process.platform', () => {
    setPlatform('win32');
    expect(localPlatform()).toBe('windows');
    setPlatform('darwin');
    expect(localPlatform()).toBe('mac');
    setPlatform('linux');
    expect(localPlatform()).toBe('posix');
  });

  it('says unknown without a preload, or with a platform that is not a name', () => {
    setPlatform(undefined);
    expect(localPlatform()).toBe('unknown');
    setPlatform('');
    expect(localPlatform()).toBe('unknown');
    setPlatform(42);
    expect(localPlatform()).toBe('unknown');
  });
});

// W2-UIW-13: the daemon checks an Identity file with Rust's `Path::is_absolute` for the OS it runs
// on, which is this computer's. These are its answers, OS by OS.
describe('isLocalAbsolutePath', () => {
  const cases: Array<[string, Record<LocalPlatform, boolean>]> = [
    ['/Users/me/.ssh/id_ed25519', { windows: false, mac: true, posix: true, unknown: true }],
    ['C:\\Users\\me\\.ssh\\id_ed25519', { windows: true, mac: false, posix: false, unknown: true }],
    ['C:/Users/me/.ssh/id_ed25519', { windows: true, mac: false, posix: false, unknown: true }],
    ['d:\\keys\\lab', { windows: true, mac: false, posix: false, unknown: true }],
    ['\\\\fileserver\\home\\me\\id', { windows: true, mac: false, posix: false, unknown: true }],
    ['\\\\?\\C:\\Users\\me\\id', { windows: true, mac: false, posix: false, unknown: true }],
    // Forward slashes count as separators in a Windows prefix; on a Mac this is just rooted.
    ['//fileserver/home/me/id', { windows: true, mac: true, posix: true, unknown: true }],
    // Never expanded, anywhere.
    ['~/.ssh/id_ed25519', { windows: false, mac: false, posix: false, unknown: false }],
    ['~\\.ssh\\id_ed25519', { windows: false, mac: false, posix: false, unknown: false }],
    ['id_ed25519', { windows: false, mac: false, posix: false, unknown: false }],
    ['.ssh/id_ed25519', { windows: false, mac: false, posix: false, unknown: false }],
    // Not absolute on Windows: rooted without a drive, and a drive without a root.
    ['\\Users\\me\\id', { windows: false, mac: false, posix: false, unknown: false }],
    ['C:Users\\me\\id', { windows: false, mac: false, posix: false, unknown: false }],
  ];
  for (const [path, expected] of cases) {
    it(`judges ${path} as the daemon does on each OS`, () => {
      for (const platform of Object.keys(expected) as LocalPlatform[]) {
        expect([platform, isLocalAbsolutePath(path, platform)]).toEqual([
          platform,
          expected[platform],
        ]);
      }
    });
  }
});
