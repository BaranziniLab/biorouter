import {
  describe,
  it,
  expect,
  beforeAll,
  afterAll,
  beforeEach,
  afterEach,
  vi,
  type MockInstance,
} from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {
  canonicalizeForContainment,
  isFilePathAllowedForPreview,
  isPathContained,
  isSensitivePreviewPath,
  reachesUnchosenNetworkHost,
} from './pathContainment';

const protectedSystemPath =
  process.platform === 'win32'
    ? path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'drivers', 'etc', 'hosts')
    : '/etc/hosts';

/**
 * Gate for the live false-denial: a session tool call wrote /tmp/qa-r1b/hi.txt
 * and the preview panel refused it with "Access denied: path '/tmp/qa-r1b/hi.txt'
 * is outside allowed directories" — because on macOS /tmp is a symlink to
 * /private/tmp and the old check was a plain string-prefix with no symlink
 * resolution. These tests exercise real symlinks in a real tempdir.
 */
describe('isPathContained', () => {
  let real: string; // a real dir, canonical
  let alias: string; // a symlink pointing at it

  beforeAll(() => {
    real = fs.mkdtempSync(path.join(fs.realpathSync(os.tmpdir()), 'pc-real-'));
    alias = path.join(fs.realpathSync(os.tmpdir()), `pc-alias-${process.pid}`);
    fs.symlinkSync(real, alias);
    fs.writeFileSync(path.join(real, 'hi.txt'), 'hello');
  });

  afterAll(() => {
    fs.unlinkSync(alias);
    fs.rmSync(real, { recursive: true, force: true });
  });

  it('admits a file addressed through a symlinked alias of an allowed root (the /tmp case)', () => {
    // Root registered in canonical form, file addressed via the alias.
    expect(isPathContained(path.join(alias, 'hi.txt'), [real])).toBe(true);
  });

  it('admits a file addressed canonically when the ROOT is registered via its alias', () => {
    // Root registered as the alias (like allowing '/tmp'), file canonical.
    expect(isPathContained(path.join(real, 'hi.txt'), [alias])).toBe(true);
  });

  it('admits a not-yet-existing write target under an allowed root', () => {
    expect(isPathContained(path.join(alias, 'new-dir', 'out.csv'), [real])).toBe(true);
  });

  it('denies a path outside every root', () => {
    expect(isPathContained('/etc/passwd', [real])).toBe(false);
  });

  it('denies lexical traversal escaping a root', () => {
    // canonicalize collapses the ../.. of the deepest existing ancestor.
    expect(isPathContained(path.join(real, '..', '..', 'etc', 'passwd'), [real])).toBe(false);
  });

  it('denies a sibling whose name merely extends the root string', () => {
    // /tmp/pc-real-XXXXevil must not match root /tmp/pc-real-XXXX.
    expect(isPathContained(`${real}evil/secret.txt`, [real])).toBe(false);
  });

  it('follows a symlink INSIDE a root that points outside it (false-admit guard)', () => {
    const outside = fs.mkdtempSync(path.join(fs.realpathSync(os.tmpdir()), 'pc-outside-'));
    const inner = path.join(real, 'escape');
    fs.symlinkSync(outside, inner);
    try {
      expect(isPathContained(path.join(inner, 'x.txt'), [real])).toBe(false);
    } finally {
      fs.unlinkSync(inner);
      fs.rmSync(outside, { recursive: true, force: true });
    }
  });

  it('canonicalizeForContainment resolves an existing symlink and preserves a missing tail', () => {
    expect(canonicalizeForContainment(alias)).toBe(real);
    expect(canonicalizeForContainment(path.join(alias, 'nope', 'deep.txt'))).toBe(
      path.join(real, 'nope', 'deep.txt')
    );
  });
});

/**
 * Directive 2 — the preview allowlist is mode-aware, but a small sensitive set
 * stays denied in EVERY mode.
 */
describe('isSensitivePreviewPath', () => {
  const home = os.homedir();

  it('denies protected system directories', () => {
    const paths =
      process.platform === 'win32'
        ? [
            protectedSystemPath,
            path.join(process.env.ProgramFiles || 'C:\\Program Files', 'Biorouter', 'x'),
            path.join(process.env.ProgramData || 'C:\\ProgramData', 'Biorouter', 'x'),
          ]
        : ['/etc/hosts', '/usr/bin/x', '/bin/ls', '/System/Library/x', '/Library/y'];
    for (const p of paths) {
      expect(isSensitivePreviewPath(p)).toBe(true);
    }
  });

  it('denies credential and persistence paths under home', () => {
    for (const rel of [
      '.ssh/id_rsa',
      '.aws/credentials',
      '.gnupg/secring.gpg',
      'Library/Keychains/login.keychain-db',
      'Library/Application Support/Google/Chrome/Default/Login Data',
    ]) {
      expect(isSensitivePreviewPath(path.join(home, rel))).toBe(true);
    }
  });

  it('allows ordinary files, temp scratch, and workspace outputs', () => {
    expect(isSensitivePreviewPath(path.join(home, 'Documents', 'notes.txt'))).toBe(false);
    expect(isSensitivePreviewPath(path.join(home, 'project', 'out.csv'))).toBe(false);
    expect(isSensitivePreviewPath(path.join(os.tmpdir(), 'scratch.txt'))).toBe(false);
    expect(isSensitivePreviewPath('/tmp/qa/hi.txt')).toBe(false);
  });
});

/**
 * AG-F6: the temp-tree exemption ran before the home-folder deny, so with HOME
 * under `/tmp`, `$TMPDIR` or `/var/folders` every credential folder in it was
 * "scratch" and previewable in every mode. The QA stage's profiles live under
 * `/private/tmp`, and the panel opened `~/.ssh/config` in full.
 */
describe('isSensitivePreviewPath with HOME inside a temp tree', () => {
  let tempHome: string;
  let homedir: MockInstance<typeof os.homedir>;
  const credentialPaths = [
    ['.ssh', 'config'],
    ['.ssh', 'id_ed25519'],
    ['.aws', 'credentials'],
    ['Library', 'Keychains', 'x'],
    ['.gnupg', 'pubring.kbx'],
  ];

  beforeEach(() => {
    // The temp dir as the OS spells it (on macOS `/var/folders/...`, a link to
    // `/private/var/folders/...`), so both the given and canonical homes are
    // exercised.
    tempHome = fs.mkdtempSync(path.join(os.tmpdir(), 'pc-home-'));
    for (const parts of credentialPaths) {
      fs.mkdirSync(path.join(tempHome, ...parts.slice(0, -1)), { recursive: true });
      fs.writeFileSync(path.join(tempHome, ...parts), 'not a real credential');
    }
    homedir = vi.spyOn(os, 'homedir').mockReturnValue(tempHome);
  });

  afterEach(() => {
    homedir.mockRestore();
    fs.rmSync(tempHome, { recursive: true, force: true });
  });

  it('denies the credential folders in normal and Fully-Automatic mode', () => {
    for (const home of [tempHome, fs.realpathSync(tempHome)]) {
      for (const parts of credentialPaths) {
        const candidate = path.join(home, ...parts);
        expect(isSensitivePreviewPath(candidate), candidate).toBe(true);
        for (const fullyAutomatic of [false, true]) {
          expect(
            isFilePathAllowedForPreview(candidate, [home, os.tmpdir(), '/tmp'], {
              fullyAutomatic,
            }),
            `${candidate} (fullyAutomatic: ${fullyAutomatic})`
          ).toBe(false);
        }
      }
    }
  });

  it('keeps the rest of that home, and the temp tree, previewable', () => {
    const notes = path.join(tempHome, 'notes.txt');
    fs.writeFileSync(notes, 'hello');
    expect(isSensitivePreviewPath(notes)).toBe(false);
    expect(
      isFilePathAllowedForPreview(notes, [tempHome, os.tmpdir()], { fullyAutomatic: false })
    ).toBe(true);
    expect(isSensitivePreviewPath(path.join(os.tmpdir(), 'scratch.txt'))).toBe(false);
  });
});

describe('isFilePathAllowedForPreview', () => {
  const home = os.homedir();

  it('in a non-auto mode, only contained non-sensitive paths are allowed', () => {
    expect(
      isFilePathAllowedForPreview(path.join(home, 'a.txt'), [home], { fullyAutomatic: false })
    ).toBe(true);
    // Outside the allowed roots → denied when not fully automatic.
    expect(isFilePathAllowedForPreview('/data/out.csv', [home], { fullyAutomatic: false })).toBe(
      false
    );
    // Sensitive even though it would be "contained" → denied.
    expect(
      isFilePathAllowedForPreview(protectedSystemPath, [home], { fullyAutomatic: false })
    ).toBe(false);
  });

  it('in Fully-Automatic mode, any non-sensitive path is allowed (parity with backend)', () => {
    expect(isFilePathAllowedForPreview('/data/out.csv', [home], { fullyAutomatic: true })).toBe(
      true
    );
    expect(
      isFilePathAllowedForPreview('/srv/results/plot.png', [home], { fullyAutomatic: true })
    ).toBe(true);
  });

  it('keeps sensitive paths denied even in Fully-Automatic mode', () => {
    expect(isFilePathAllowedForPreview(protectedSystemPath, [home], { fullyAutomatic: true })).toBe(
      false
    );
    expect(
      isFilePathAllowedForPreview(path.join(home, '.ssh', 'id_rsa'), [home], {
        fullyAutomatic: true,
      })
    ).toBe(false);
  });
});

/**
 * On Windows a path that starts with two separators (`\\host\share\…`,
 * `//host/…`, `\\?\UNC\…`) is a network path, and the `realpath` and `lstat`
 * the file read starts with open an SMB connection to that host, which may try
 * to sign in to it. The renderer asks for previews of paths a model wrote, so a
 * model could make the app reach a host of its choosing with no click (review
 * finding on OP-P0-09, round 1). Such a path is refused before the filesystem is
 * touched, unless it lies on a share the person put in scope.
 */
describe('network paths on Windows', () => {
  const roots = ['C:\\Users\\ada', 'C:\\Users\\ada\\AppData\\Local\\Temp'];
  let realpath: MockInstance<typeof fs.realpathSync>;
  let lstat: MockInstance<typeof fs.lstatSync>;

  beforeEach(() => {
    realpath = vi.spyOn(fs, 'realpathSync');
    lstat = vi.spyOn(fs, 'lstatSync');
  });

  afterEach(() => {
    realpath.mockRestore();
    lstat.mockRestore();
  });

  it.each([
    '\\\\evil.test\\share\\a.png',
    '//evil.test/share/a.png',
    '\\\\evil.test\\IPC$',
    '\\\\?\\UNC\\evil.test\\share\\a.png',
    '\\\\.\\UNC\\evil.test\\share\\a.png',
  ])('refuses %s in every mode without touching the filesystem', (candidate) => {
    for (const fullyAutomatic of [false, true]) {
      expect(
        isFilePathAllowedForPreview(candidate, roots, { fullyAutomatic, platform: 'win32' })
      ).toBe(false);
    }
    expect(realpath).not.toHaveBeenCalled();
    expect(lstat).not.toHaveBeenCalled();
  });

  it('names a network host nobody chose', () => {
    expect(reachesUnchosenNetworkHost('\\\\evil.test\\share\\a.png', roots, 'win32')).toBe(true);
    expect(reachesUnchosenNetworkHost('//evil.test/a.png', roots, 'win32')).toBe(true);
  });

  it('keeps a file on the share the person works in', () => {
    const share = ['\\\\lab-server\\share\\proj', ...roots];
    expect(reachesUnchosenNetworkHost('\\\\lab-server\\share\\proj\\fig.png', share, 'win32')).toBe(
      false
    );
    expect(reachesUnchosenNetworkHost('//LAB-SERVER/share/proj/out/fig.png', share, 'win32')).toBe(
      false
    );
    expect(reachesUnchosenNetworkHost('\\\\lab-server\\share\\proj', share, 'win32')).toBe(false);
    // Another share, a sibling whose name extends the folder's, or a way out of it.
    expect(reachesUnchosenNetworkHost('\\\\lab-server\\other\\fig.png', share, 'win32')).toBe(true);
    expect(
      reachesUnchosenNetworkHost('\\\\lab-server\\share\\proj2\\fig.png', share, 'win32')
    ).toBe(true);
    expect(
      reachesUnchosenNetworkHost(
        '\\\\lab-server\\share\\proj\\..\\..\\other\\fig.png',
        share,
        'win32'
      )
    ).toBe(true);
  });

  it('leaves drive paths alone', () => {
    expect(reachesUnchosenNetworkHost('C:\\Users\\ada\\fig.png', roots, 'win32')).toBe(false);
    expect(reachesUnchosenNetworkHost('Z:\\mapped\\fig.png', roots, 'win32')).toBe(false);
  });

  it('leaves POSIX paths alone: two leading slashes name the root there', () => {
    for (const platform of ['darwin', 'linux'] as const) {
      expect(reachesUnchosenNetworkHost('//evil.test/a.png', [], platform)).toBe(false);
    }
  });
});
