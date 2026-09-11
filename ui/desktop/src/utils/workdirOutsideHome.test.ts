import os from 'node:os';
import path from 'node:path';
import fsSync from 'node:fs';
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { isFilePathAllowedForPreview, previewFileRoots } from './pathContainment';
import { expandTilde, reinterpretTildeAsAbsolute } from './pathUtils';

/**
 * A chat whose working directory is OUTSIDE the home tree.
 *
 * Reported from the field: working directory `/ws/projects/ucsf/ic`, Biorouter
 * writing files there correctly, and then the artifact panel answering
 * "Access denied: … is outside allowed directories" for those same files —
 * while a second symptom, a `~/` glued onto the front of one of the paths,
 * made a different link point at a file that never existed.
 *
 * Both are exercised here against a real directory outside `$HOME`, because
 * that is the one condition under which either reproduces.
 */
describe('a working directory outside the home tree', () => {
  let outside: string;
  let file: string;

  beforeAll(() => {
    // ⚠ `os.tmpdir()` and `/tmp` are allowed roots in their OWN right, so a
    // directory built there would be admitted whether or not the fix exists and
    // the test would pass vacuously. This needs somewhere genuinely outside
    // every pre-existing root — the situation the report describes, where the
    // user's folder is `/ws/projects/ucsf/ic`.
    //
    // `/Users/Shared` (macOS) and `/var/tmp` (elsewhere) are writable, real, and
    // under none of home / userData / temp / tmpdir / `/tmp`. Note `/var/tmp` is
    // NOT `/var/folders`, which `isTempPreviewPath` exempts.
    const outsideBase = process.platform === 'darwin' ? '/Users/Shared' : '/var/tmp';
    outside = fsSync.mkdtempSync(path.join(outsideBase, 'br-workdir-'));
    file = path.join(outside, 'figure.png');
    fsSync.writeFileSync(file, 'x');
  });

  afterAll(() => {
    fsSync.rmSync(outside, { recursive: true, force: true });
  });

  const NARROW = { fullyAutomatic: false };
  // The roots that existed before the fix: home, userData, temp, tmpdir, /tmp.
  const rootsWithoutWorkingDir = [os.homedir(), os.tmpdir(), '/tmp'];

  it('is denied when the chosen folder is not an allowed root', () => {
    // The bug, stated as a fact about the old root set.
    expect(isFilePathAllowedForPreview(file, rootsWithoutWorkingDir, NARROW)).toBe(false);
  });

  it('is allowed once the session working directory is a root', () => {
    // ⚠ Fails without the fix: `allowedFileRoots()` took no argument, so the
    // folder the user pointed the chat at could never appear in this list.
    expect(isFilePathAllowedForPreview(file, [outside, ...rootsWithoutWorkingDir], NARROW)).toBe(
      true
    );
  });

  it('does not widen to a sensitive path even when it is the working directory', () => {
    // The widening adds a root; it must not bypass the sensitive-path deny.
    const ssh = path.join(os.homedir(), '.ssh', 'id_rsa');
    expect(isFilePathAllowedForPreview(ssh, [path.join(os.homedir(), '.ssh')], NARROW)).toBe(false);
  });

  it('does not admit an unrelated sibling of the working directory', () => {
    // Containment, not prefix-matching on the parent.
    const sibling = `${outside}-other`;
    expect(isFilePathAllowedForPreview(sibling, [outside], NARROW)).toBe(false);
  });

  /**
   * The ROOT SET itself. The containment assertions above hand-build their roots
   * array, so they pass whether or not the working directory is ever actually
   * added — they check the predicate, not the wiring. These check the wiring.
   */
  describe('the root set main.ts builds', () => {
    const base = {
      home: '/Users/someone',
      userData: '/Users/someone/Library/Application Support/Biorouter',
      appTemp: '/var/folders/xx/T/biorouter',
      systemTemp: '/var/folders/xx/T',
      platform: 'darwin' as typeof process.platform,
    };

    it('includes the session working directory', () => {
      // ⚠ Fails without the fix: `allowedFileRoots()` took no argument at all.
      expect(previewFileRoots({ ...base, sessionWorkingDir: '/ws/projects/ucsf/ic' })).toContain(
        '/ws/projects/ucsf/ic'
      );
    });

    it('omits it when the chat has none, rather than inventing one', () => {
      const roots = previewFileRoots(base);
      expect(roots).not.toContain(undefined);
      expect(roots).toEqual([base.home, base.userData, base.appTemp, base.systemTemp, '/tmp']);
    });

    it('keeps every pre-existing root', () => {
      // The widening must ADD, never replace: a regression here would break
      // previewing in home or /tmp, which is most sessions.
      const roots = previewFileRoots({ ...base, sessionWorkingDir: '/ws' });
      for (const r of [base.home, base.userData, base.appTemp, base.systemTemp, '/tmp']) {
        expect(roots).toContain(r);
      }
    });

    it('drops /tmp on Windows and still carries the working directory', () => {
      const roots = previewFileRoots({
        ...base,
        platform: 'win32' as typeof process.platform,
        sessionWorkingDir: 'D:\\ws',
      });
      expect(roots).not.toContain('/tmp');
      expect(roots).toContain('D:\\ws');
    });

    it('still honours BIOROUTER_PATH_ROOT', () => {
      expect(previewFileRoots({ ...base, pathRootOverride: '/tmp/root' })).toContain('/tmp/root');
    });
  });

  describe('a ~/ the model glued onto a non-home path', () => {
    const exists = (p: string) => fsSync.existsSync(p);

    it('resolves to the real file instead of a home path that is not there', () => {
      // `~/<outside>/figure.png` — exactly the shape in the report.
      const written = `~${file}`;
      expect(expandTilde(written)).toBe(path.join(os.homedir(), file.slice(1)));
      expect(fsSync.existsSync(expandTilde(written))).toBe(false);
      // ⚠ Fails without the fix: the dead home path was the final answer.
      expect(reinterpretTildeAsAbsolute(written, expandTilde(written), exists)).toBe(file);
    });

    it('leaves a genuine home path alone', () => {
      // The control. A `~/` path that DOES exist under home must not move.
      const real = '~/';
      const expanded = expandTilde(real);
      expect(reinterpretTildeAsAbsolute(real, expanded, exists)).toBe(expanded);
    });

    it('keeps the home reading when both exist', () => {
      // Precedence: never redirect a path that already works.
      expect(
        reinterpretTildeAsAbsolute('~/x', '/home-x', (p) => p === '/home-x' || p === '/x')
      ).toBe('/home-x');
    });

    it('keeps the home reading when neither exists', () => {
      // No invention: an unresolvable path stays as it was, so the caller still
      // reports a missing file rather than a surprising one.
      expect(reinterpretTildeAsAbsolute('~/nope', '/home-nope', () => false)).toBe('/home-nope');
    });

    it('ignores a path with no tilde at all', () => {
      expect(reinterpretTildeAsAbsolute('/ws/a', '/ws/a', () => false)).toBe('/ws/a');
    });
  });
});
