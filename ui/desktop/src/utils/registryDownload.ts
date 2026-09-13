/**
 * Where a downloaded marketplace asset is staged before the daemon imports it.
 *
 * ⚠ **The collision nonce goes in the DIRECTORY, never in the filename.**
 * Two downloads that share a basename must not clobber each other in a shared
 * temp directory, so a nonce is needed — but the filename is not a private
 * detail of this process. `POST /skills/packages/install` takes the staged
 * path, and the importer reads the archive's **stem** as a candidate package id
 * (`skill_package::source::archive_stem` → `plan::resolve_identity`) for any
 * package whose archive declares no name of its own. Every BAAM bundle asset is
 * such a package: `single-cell.zip` carries no `skills-manifest.json`, so the
 * stem IS the identity.
 *
 * Staging it as `<nonce>-single-cell.zip` therefore installed the bundle into
 * `~/.config/biorouter/skills/d92c1c985d54-single-cell/` and wrote that string
 * into `biorouter-package.json` as both `id` and `displayName` — so Settings
 * listed the package under a random hex prefix, `searchSkills` reported that as
 * its bundle name, and Browse skills (which compares against the registry id
 * `single-cell`) could never match it and re-offered the same bundle forever. A
 * single skill escaped only because `plan::single_plan` prefers the SKILL.md
 * frontmatter name over the stem.
 *
 * The nonce is `crypto.randomBytes(6)` — a staging nonce, not a content hash
 * and not a package identity: the same asset downloaded twice gets two
 * different prefixes. Nothing about it is worth carrying into the installed
 * name. Giving each download its own directory keeps the collision property
 * intact — strictly stronger, since the whole staging area is now private to
 * one download — and hands the importer the asset's real stem.
 *
 * Electron-free on purpose, for the reason `registryCache.ts` states: `main.ts`
 * imports `electron` at the top level and cannot be unit-tested, and the
 * renderer's tests stop at the IPC boundary.
 */

import path from 'node:path';

/** The staging root every marketplace download lands under. */
export const REGISTRY_STAGING_DIR = 'biorouter-registry';

/**
 * The directory and file one download should be written to.
 *
 * `pathname` is the asset URL's path; `nonce` is the caller's random hex. The
 * basename is sanitised exactly as before (anything outside `[A-Za-z0-9._-]`
 * becomes `_`), falling back to `asset<ext>` when the URL carries no usable
 * one. The returned `file` is asserted to sit directly inside `dir`, because
 * this value decides where bytes are written.
 */
export function stagedAssetPath(args: {
  tmpDir: string;
  pathname: string;
  nonce: string;
  ext: string;
}): { dir: string; file: string } {
  const { tmpDir, pathname, nonce, ext } = args;
  const sanitized = path.basename(pathname).replace(/[^a-zA-Z0-9._-]/g, '_');
  const safeName = sanitized && sanitized !== '.' && sanitized !== '..' ? sanitized : `asset${ext}`;
  const dir = path.join(tmpDir, REGISTRY_STAGING_DIR, nonce);
  const file = path.join(dir, safeName);
  if (path.dirname(file) !== dir) {
    throw new Error(`refusing to stage a registry asset outside ${dir}`);
  }
  return { dir, file };
}
