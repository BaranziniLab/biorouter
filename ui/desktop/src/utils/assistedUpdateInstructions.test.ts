import { describe, expect, it } from 'vitest';
import { assistedUpdateInstructions, assistedUpdateKind } from './assistedUpdateInstructions';

// The real candidate names, from `githubUpdater.ts`. If those drift, these
// fixtures should drift with them.
const WIN_INSTALLER =
  'C:\\Users\\x\\AppData\\Roaming\\Biorouter\\updates\\Biorouter-Setup-1.91.0.exe';
const WIN_ZIP =
  'C:\\Users\\x\\AppData\\Roaming\\Biorouter\\updates\\Biorouter-win32-x64-1.91.0.zip';
const LINUX_DEB = '/home/x/.cache/biorouter/updates/biorouter_1.91.0_amd64.deb';
const MAC_APP = '/Users/x/Library/Caches/biorouter/updates/Biorouter.app';

describe('assistedUpdateKind', () => {
  it('tells the two Windows artifacts apart by extension', () => {
    expect(assistedUpdateKind(WIN_INSTALLER, 'win32')).toBe('windows-installer');
    expect(assistedUpdateKind(WIN_ZIP, 'win32')).toBe('windows-zip');
  });

  it('is case-insensitive, because a server may hand back any casing', () => {
    expect(assistedUpdateKind(WIN_INSTALLER.toUpperCase(), 'win32')).toBe('windows-installer');
  });

  it('does not misread a .exe on a platform that never ships one', () => {
    expect(assistedUpdateKind('/tmp/weird.exe', 'linux')).toBe('linux-package');
    expect(assistedUpdateKind('/tmp/weird.exe', 'darwin')).toBe('macos-app');
  });
});

describe('assistedUpdateInstructions', () => {
  // This is the regression. The dialog branched on process.platform, so a
  // Windows user who downloaded Biorouter-Setup-<ver>.exe was told to extract a
  // zip that was never downloaded and to replace a folder by hand that Squirrel
  // replaces for them.
  it('does not tell an installer user to extract a zip', () => {
    const text = assistedUpdateInstructions(WIN_INSTALLER, 'win32');
    expect(text).not.toMatch(/zip/i);
    expect(text).not.toMatch(/replace your existing Biorouter folder/i);
    expect(text).toMatch(/Run the installer/i);
  });

  it('still gives the manual steps for a release that only shipped the zip', () => {
    const text = assistedUpdateInstructions(WIN_ZIP, 'win32');
    expect(text).toMatch(/Extract the zip/i);
    expect(text).not.toMatch(/Run the installer/i);
  });

  it('never names a macOS concept on Windows or Linux', () => {
    for (const [path, platform] of [
      [WIN_INSTALLER, 'win32'],
      [WIN_ZIP, 'win32'],
      [LINUX_DEB, 'linux'],
    ] as const) {
      const text = assistedUpdateInstructions(path, platform);
      expect(text, `${platform} ${path}`).not.toMatch(/\.app|Applications folder/i);
    }
  });

  it('keeps the macOS steps unchanged', () => {
    const text = assistedUpdateInstructions(MAC_APP, 'darwin');
    expect(text).toMatch(/Drag the new Biorouter\.app to your Applications folder/);
  });

  it('promises data is preserved on every platform, since that is what the user is deciding on', () => {
    for (const [path, platform] of [
      [WIN_INSTALLER, 'win32'],
      [WIN_ZIP, 'win32'],
      [LINUX_DEB, 'linux'],
    ] as const) {
      expect(assistedUpdateInstructions(path, platform)).toMatch(/preserved/i);
    }
  });

  it('every step list is numbered from 1 with no gaps', () => {
    for (const [path, platform] of [
      [WIN_INSTALLER, 'win32'],
      [WIN_ZIP, 'win32'],
      [LINUX_DEB, 'linux'],
      [MAC_APP, 'darwin'],
    ] as const) {
      const steps = assistedUpdateInstructions(path, platform)
        .split('\n')
        .map((line) => /^(\d+)\./.exec(line)?.[1])
        .filter(Boolean)
        .map(Number);
      expect(steps, `${platform} ${path}`).toEqual(steps.map((_, i) => i + 1));
    }
  });
});
