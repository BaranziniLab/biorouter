/**
 * Which release the extension updater may install for one installed extension,
 * decided here and applied by `extensionUpdater.ts`.
 *
 * ⚠ **Why this exists.** The updater used to take the FIRST asset whose name
 * ends in `.brxt` from `releases/latest` of whatever repository the installed
 * manifest named, and extracted it over the install with no name, digest or
 * manifest check. GitHub lists release assets in case-insensitive NAME order,
 * not upload order, so `gptr.brxt` and `gptr-1.4.0.brxt` come before
 * `gptragent.brxt` in one release, and a `gptragent` install would have been
 * overwritten by a bundle whose entry point its configuration does not run.
 * The rules now:
 *
 *   1. A folder under an id that was renamed away (`extensions/gptragent`) is
 *      left alone: the daemon moves it to the new id at start, and an update
 *      written into the old folder would be moved or orphaned under it.
 *   2. An extension BAAM lists follows its registry row: the row's version, the
 *      row's own download address (a GitHub release download, so a never-latest
 *      prerelease is reachable by its tag) and the SHA-256 digest GitHub
 *      publishes for that asset. `registry.json` v2 has no digest field and
 *      cannot gain one (`marketplace.rs` parses it with `deny_unknown_fields`),
 *      so the digest comes from GitHub's release API.
 *   3. Any other extension takes exactly one asset named `<name>.brxt` from
 *      `releases/latest`, and only with its published digest.
 *   4. After the download, `checkBundleManifest` holds the bundle to the install:
 *      the same extension under one of its ids, the same entry point (the
 *      config's `args` end in it), and the release's version (else the updater
 *      would install it again on every launch).
 *
 * Nothing here reads the disk or the network; the caller passes `fetchJson`.
 */

import { compareVersions } from 'compare-versions';
import { allExtensionKeys, currentExtensionKey, isRenamedAway } from './extensionRenames';

export const GITHUB_API_REPOS = 'https://api.github.com/repos';

/** What the updater reads from an installed extension's `manifest.json`. */
export interface InstalledManifest {
  name: string;
  version: string;
  entry_point?: string;
  repository?: string;
}

/** A release the updater may install, with the digest the download must match. */
export interface ChosenRelease {
  /** `registry`: the BAAM row named it. `latest`: `releases/latest` of the manifest repository. */
  from: 'registry' | 'latest';
  /** Without a leading `v`. The bundle's manifest must declare this version. */
  version: string;
  tag: string;
  assetName: string;
  downloadUrl: string;
  /** 64 lowercase hex digits. */
  sha256: string;
  prerelease: boolean;
}

export type ReleaseDecision =
  | { kind: 'update'; release: ChosenRelease }
  | { kind: 'up-to-date'; version: string }
  /** Nothing to do and nothing wrong: an old-id folder, or no GitHub repository to ask. */
  | { kind: 'skip'; reason: string }
  /** A release may exist, but it cannot be installed safely. Logged; nothing changes. */
  | { kind: 'refuse'; reason: string };

export type FetchJson = (url: string) => Promise<unknown>;

/** A BAAM `registry.json` extension row, as far as the updater reads it. */
export interface RegistryRow {
  id: string;
  version?: unknown;
  download?: unknown;
  filename?: unknown;
  extension_name?: unknown;
}

interface GitHubAsset {
  name: string;
  browser_download_url?: unknown;
  digest?: unknown;
}

interface GitHubRelease {
  tag_name: string;
  prerelease: boolean;
  assets: GitHubAsset[];
}

/**
 * `config::extensions::name_to_key`: whitespace removed, lowercased. The key the
 * daemon files an extension under, and the form `registry.json` publishes
 * `extension_name` in.
 */
export function extensionKey(name: string): string {
  return name.replace(/\s/gu, '').toLowerCase();
}

/** A version string without whitespace or a leading `v`. */
export function cleanVersion(version: string): string {
  return version.trim().replace(/^v/i, '');
}

/** `candidate` is a later version than `current`. False when either does not parse. */
export function isNewerVersion(candidate: string, current: string): boolean {
  try {
    return compareVersions(cleanVersion(candidate), cleanVersion(current)) > 0;
  } catch {
    return false;
  }
}

/**
 * The two strings name one version. Semantic, not textual: a row says
 * `v2.0.0+biorouter.1` where a manifest may say `2.0.0+biorouter.1`, and what
 * the check prevents is a bundle the next launch would see as older or newer
 * than the release, i.e. installed again on every launch.
 */
export function sameVersion(a: string, b: string): boolean {
  try {
    return compareVersions(cleanVersion(a), cleanVersion(b)) === 0;
  } catch {
    return cleanVersion(a) === cleanVersion(b);
  }
}

/** `{ owner, repo }` of a `https://github.com/<owner>/<repo>` address, or null. */
export function parseGitHubRepo(repoUrl: unknown): { owner: string; repo: string } | null {
  if (typeof repoUrl !== 'string') return null;
  try {
    const url = new URL(repoUrl);
    if (url.hostname !== 'github.com' && url.hostname !== 'www.github.com') return null;
    const [owner, rawRepo] = url.pathname.replace(/^\//, '').replace(/\/$/, '').split('/');
    const repo = rawRepo?.replace(/\.git$/, '');
    if (!isRepoSegment(owner) || !isRepoSegment(repo)) return null;
    return { owner, repo };
  } catch {
    return null;
  }
}

/** A GitHub owner or repository name, safe to place in an API path as is. */
function isRepoSegment(segment: string | undefined): segment is string {
  return !!segment && /^[A-Za-z0-9_.-]+$/.test(segment) && segment !== '.' && segment !== '..';
}

/** The hex of a `sha256:<64 hex>` digest, lowercased, or null for anything else. */
export function sha256FromDigest(digest: unknown): string | null {
  if (typeof digest !== 'string') return null;
  const match = /^sha256:([0-9a-f]{64})$/i.exec(digest.trim());
  return match ? match[1].toLowerCase() : null;
}

/**
 * The owner, repository and tag of a GitHub release download address
 * (`https://github.com/<owner>/<repo>/releases/download/<tag>/<filename>`),
 * when its last segment is exactly `filename`. Anything else, a query or a
 * fragment included, is null: the registry row is the whole instruction, so
 * nothing in it may be read two ways.
 */
export function parseReleaseDownload(
  download: unknown,
  filename: unknown
): { owner: string; repo: string; tag: string } | null {
  if (typeof download !== 'string' || typeof filename !== 'string' || !filename) return null;
  let url: URL;
  try {
    url = new URL(download);
  } catch {
    return null;
  }
  if (
    url.protocol !== 'https:' ||
    url.hostname !== 'github.com' ||
    url.port ||
    url.username ||
    url.password ||
    url.search ||
    url.hash
  ) {
    return null;
  }
  const parts = url.pathname.split('/');
  if (parts.length !== 7 || parts[0] !== '') return null;
  const [, owner, repo, releases, downloadSegment, rawTag, rawFile] = parts;
  if (releases !== 'releases' || downloadSegment !== 'download') return null;
  if (!isRepoSegment(owner) || !isRepoSegment(repo)) return null;
  let tag: string;
  let file: string;
  try {
    tag = decodeURIComponent(rawTag);
    file = decodeURIComponent(rawFile);
  } catch {
    return null;
  }
  if (!tag || file !== filename) return null;
  return { owner, repo, tag };
}

/**
 * Every row of `registry` whose installed name (`extension_name`, or the `id`
 * when a row has none) is one of the ids the extension named `manifestName`
 * has had, so a row for `gptragent` matches an install of `gptr` and the other
 * way round. A document that is not a catalogue yields no rows.
 */
export function registryRowsFor(registry: unknown, manifestName: string): RegistryRow[] {
  const extensions = (registry as { extensions?: unknown } | null)?.extensions;
  if (!Array.isArray(extensions)) return [];
  const keys = allExtensionKeys(manifestName);
  return extensions.filter((row): row is RegistryRow => {
    if (!row || typeof row !== 'object' || Array.isArray(row)) return false;
    const { id, extension_name: extensionName } = row as RegistryRow;
    if (typeof id !== 'string') return false;
    const name = typeof extensionName === 'string' ? extensionName : id;
    return keys.includes(extensionKey(name));
  });
}

function asRelease(value: unknown): GitHubRelease | null {
  if (!value || typeof value !== 'object') return null;
  const { tag_name: tag, prerelease, assets } = value as Record<string, unknown>;
  if (typeof tag !== 'string' || !Array.isArray(assets)) return null;
  return {
    tag_name: tag,
    prerelease: prerelease === true,
    assets: assets.filter(
      (asset): asset is GitHubAsset =>
        !!asset && typeof asset === 'object' && typeof (asset as GitHubAsset).name === 'string'
    ),
  };
}

function isHttpsUrl(value: unknown): value is string {
  if (typeof value !== 'string') return false;
  try {
    return new URL(value).protocol === 'https:';
  } catch {
    return false;
  }
}

/** Decide which release, if any, the extension in `folderName` may be updated to. */
export async function chooseExtensionRelease(options: {
  /** The folder's name under `extensions/`. */
  folderName: string;
  manifest: InstalledManifest;
  /** The BAAM catalogue (`registry.json`), or null when none could be read. */
  registry: unknown;
  fetchJson: FetchJson;
}): Promise<ReleaseDecision> {
  const { folderName, manifest, registry, fetchJson } = options;

  if (isRenamedAway(folderName)) {
    return {
      kind: 'skip',
      reason: `${folderName} was renamed to ${currentExtensionKey(folderName)}; Biorouter moves it before it is updated`,
    };
  }

  const rows = registryRowsFor(registry, manifest.name);
  if (rows.length > 1) {
    return {
      kind: 'refuse',
      reason: `${rows.length} marketplace entries (${rows.map((row) => row.id).join(', ')}) name this extension`,
    };
  }
  if (rows.length === 1) return releaseFromRow(rows[0], manifest, fetchJson);
  return releaseFromLatest(manifest, fetchJson);
}

async function releaseFromRow(
  row: RegistryRow,
  manifest: InstalledManifest,
  fetchJson: FetchJson
): Promise<ReleaseDecision> {
  if (typeof row.version !== 'string' || !row.version.trim()) {
    return { kind: 'refuse', reason: `the marketplace entry ${row.id} states no version` };
  }
  const version = cleanVersion(row.version);
  if (!isNewerVersion(version, manifest.version)) return { kind: 'up-to-date', version };

  const source = parseReleaseDownload(row.download, row.filename);
  if (!source) {
    return {
      kind: 'refuse',
      reason: `the marketplace entry ${row.id} does not name a GitHub release download for its file`,
    };
  }
  const filename = row.filename as string;
  const release = asRelease(
    await fetchJson(
      `${GITHUB_API_REPOS}/${source.owner}/${source.repo}/releases/tags/${encodeURIComponent(source.tag)}`
    )
  );
  if (!release || release.tag_name !== source.tag) {
    return {
      kind: 'refuse',
      reason: `GitHub did not return the release ${source.tag} of ${source.owner}/${source.repo}`,
    };
  }
  const assets = release.assets.filter((asset) => asset.name === filename);
  if (assets.length !== 1) {
    return { kind: 'refuse', reason: `the release ${source.tag} has no asset named ${filename}` };
  }
  const sha256 = sha256FromDigest(assets[0].digest);
  if (!sha256) {
    return {
      kind: 'refuse',
      reason: `GitHub published no SHA-256 digest for ${filename} in ${source.tag}`,
    };
  }
  return {
    kind: 'update',
    release: {
      from: 'registry',
      version,
      tag: source.tag,
      assetName: filename,
      downloadUrl: row.download as string,
      sha256,
      prerelease: release.prerelease,
    },
  };
}

async function releaseFromLatest(
  manifest: InstalledManifest,
  fetchJson: FetchJson
): Promise<ReleaseDecision> {
  const repo = parseGitHubRepo(manifest.repository);
  if (!repo) return { kind: 'skip', reason: 'its manifest names no GitHub repository' };

  const release = asRelease(
    await fetchJson(`${GITHUB_API_REPOS}/${repo.owner}/${repo.repo}/releases/latest`)
  );
  if (!release) {
    return { kind: 'refuse', reason: `GitHub returned no release for ${repo.owner}/${repo.repo}` };
  }
  const version = cleanVersion(release.tag_name);
  if (!isNewerVersion(version, manifest.version)) return { kind: 'up-to-date', version };

  // Exactly the installed name. GitHub keeps asset names unique without regard
  // to case, so at most one asset can match.
  const wanted = `${extensionKey(manifest.name)}.brxt`;
  const assets = release.assets.filter((asset) => asset.name.toLowerCase() === wanted);
  if (assets.length !== 1) {
    return {
      kind: 'refuse',
      reason: `the release ${release.tag_name} has no asset named ${wanted}`,
    };
  }
  const [asset] = assets;
  if (!isHttpsUrl(asset.browser_download_url)) {
    return { kind: 'refuse', reason: `${asset.name} in ${release.tag_name} has no https address` };
  }
  const sha256 = sha256FromDigest(asset.digest);
  if (!sha256) {
    return {
      kind: 'refuse',
      reason: `GitHub published no SHA-256 digest for ${asset.name} in ${release.tag_name}`,
    };
  }
  return {
    kind: 'update',
    release: {
      from: 'latest',
      version,
      tag: release.tag_name,
      assetName: asset.name,
      downloadUrl: asset.browser_download_url,
      sha256,
      prerelease: release.prerelease,
    },
  };
}

/**
 * Why the bundle's `manifest.json` may not replace the installed extension, or
 * null when it may. The sentence ends up in the update-failed notice, so it is
 * written for the person reading that.
 */
export function checkBundleManifest(
  installed: InstalledManifest,
  bundle: unknown,
  releaseVersion: string
): string | null {
  if (!bundle || typeof bundle !== 'object' || Array.isArray(bundle)) {
    return 'The downloaded bundle has no readable manifest.json.';
  }
  const { name, entry_point: entryPoint, version } = bundle as Record<string, unknown>;
  if (typeof name !== 'string' || typeof entryPoint !== 'string' || typeof version !== 'string') {
    return 'The downloaded bundle does not state its name, entry point and version.';
  }
  if (!allExtensionKeys(installed.name).includes(extensionKey(name))) {
    return `The downloaded bundle is the extension ${name}, not ${installed.name}.`;
  }
  if (entryPoint !== installed.entry_point) {
    return `The downloaded bundle starts ${entryPoint}, but this install is set up to start ${installed.entry_point ?? 'something else'}.`;
  }
  if (!sameVersion(version, releaseVersion)) {
    return `The downloaded bundle says version ${version}, but the release is ${releaseVersion}.`;
  }
  return null;
}
