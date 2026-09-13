/**
 * The staged path decides the installed package's name, so it is tested
 * directly — `main.ts` imports `electron` at the top level and cannot be, which
 * is exactly how the nonce reached the filename unnoticed in the first place.
 */
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { REGISTRY_STAGING_DIR, stagedAssetPath } from './registryDownload';

const NONCE = 'd92c1c985d54';

describe('stagedAssetPath', () => {
  /**
   * The property the daemon's importer depends on: for an archive that declares
   * no name of its own — every BAAM bundle — `plan::resolve_identity` takes the
   * file's stem as the package id. The stem must therefore be the asset's, not
   * the staging area's.
   */
  it('leaves the asset name as the file stem', () => {
    const { file } = stagedAssetPath({
      tmpDir: '/tmp',
      pathname:
        '/BaranziniLab/biorouter-skills/releases/download/skill-single-cell/single-cell.zip',
      nonce: NONCE,
      ext: '.zip',
    });
    expect(path.basename(file)).toBe('single-cell.zip');
    expect(path.basename(file, '.zip')).toBe('single-cell');
  });

  /** The collision nonce is still there — one directory per download. */
  it('makes the nonce a private directory rather than a filename prefix', () => {
    const a = stagedAssetPath({
      tmpDir: '/tmp',
      pathname: '/x/single-cell.zip',
      nonce: 'aaaaaaaaaaaa',
      ext: '.zip',
    });
    const b = stagedAssetPath({
      tmpDir: '/tmp',
      pathname: '/y/single-cell.zip',
      nonce: 'bbbbbbbbbbbb',
      ext: '.zip',
    });
    expect(a.dir).toBe(path.join('/tmp', REGISTRY_STAGING_DIR, 'aaaaaaaaaaaa'));
    expect(a.dir).not.toBe(b.dir);
    // Two different assets that share a basename cannot clobber each other.
    expect(a.file).not.toBe(b.file);
    expect(path.basename(a.file)).toBe(path.basename(b.file));
    expect(path.dirname(a.file)).toBe(a.dir);
  });

  it('sanitises the basename and keeps the file inside its staging directory', () => {
    const { dir, file } = stagedAssetPath({
      tmpDir: '/tmp',
      pathname: '/x/my pack(v2).zip',
      nonce: NONCE,
      ext: '.zip',
    });
    expect(path.basename(file)).toBe('my_pack_v2_.zip');
    expect(path.dirname(file)).toBe(dir);
  });

  it('falls back to a generic name when the URL carries none', () => {
    for (const [pathname, ext, expected] of [
      ['/', '.zip', 'asset.zip'],
      ['', '.brxt', 'asset.brxt'],
    ] as const) {
      const { file } = stagedAssetPath({ tmpDir: '/tmp', pathname, nonce: NONCE, ext });
      expect(path.basename(file)).toBe(expected);
    }
  });

  /**
   * A basename that resolved to a parent directory would write outside the
   * staging area. The caller already refuses any URL whose path does not end in
   * `.zip` or `.brxt`, so this cannot be reached through the handler — it is
   * asserted because the value decides where bytes land.
   */
  it('never escapes the staging directory', () => {
    const { dir, file } = stagedAssetPath({
      tmpDir: '/tmp',
      pathname: '/x/..',
      nonce: NONCE,
      ext: '.zip',
    });
    expect(path.basename(file)).toBe('asset.zip');
    expect(path.dirname(file)).toBe(dir);
  });
});
