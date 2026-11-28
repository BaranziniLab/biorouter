// @vitest-environment node

/**
 * Which release the extension updater may install, decided without touching the
 * disk or the network (`extensionReleaseSource.ts`).
 *
 * The updater used to take the FIRST asset ending in `.brxt` from
 * `releases/latest` of whatever repository the installed manifest named, with
 * no name, digest or manifest check. GitHub lists release assets in
 * case-insensitive NAME order, not upload order, so a release that carries
 * `gptr.brxt` beside `gptragent.brxt` handed a `gptragent` install the `gptr`
 * bundle, whose entry point does not exist in the installed configuration.
 * These tests pin the rules that replaced it: a BAAM-listed extension follows
 * its registry row, anything else takes exactly `<name>.brxt`, and nothing is
 * installed without the SHA-256 GitHub published for the asset.
 */
import { describe, expect, it, vi } from 'vitest';
import {
  checkBundleManifest,
  chooseExtensionRelease,
  parseReleaseDownload,
  registryRowsFor,
  sha256FromDigest,
  type InstalledManifest,
} from './extensionReleaseSource';

const DIGEST_A = 'a'.repeat(64);
const DIGEST_B = 'b'.repeat(64);

const GPTRAGENT_INSTALLED: InstalledManifest = {
  name: 'gptragent',
  version: '1.3.0',
  entry_point: 'gptragent',
  repository: 'https://github.com/Broccolito/gptr',
};

/** The live registry's row for gptr Agent, as `registry.json` v2 publishes it. */
function gptragentRow(overrides: Record<string, unknown> = {}) {
  return {
    id: 'gptragent',
    name: 'gptr Agent',
    organization: 'Broccolito · UCSF',
    version: 'v1.4.0',
    description: '',
    tags: [],
    github: 'https://github.com/Broccolito/gptr',
    download: 'https://github.com/Broccolito/gptr/releases/download/bridge-v1.4.0/gptragent.brxt',
    filename: 'gptragent.brxt',
    privacy: 'private',
    extension_name: 'gptragent',
    ...overrides,
  };
}

function registryOf(...extensions: unknown[]) {
  return { version: 2, extensions, skills: [] };
}

/** A GitHub release as the REST API returns it, trimmed to what the updater reads. */
function release(
  tag: string,
  assets: Array<{ name: string; digest?: string | null; url?: string }>,
  extra: Record<string, unknown> = {}
) {
  return {
    tag_name: tag,
    prerelease: false,
    draft: false,
    ...extra,
    assets: assets.map(({ name, digest, url }) => ({
      name,
      browser_download_url: url ?? `https://github.com/owner/repo/releases/download/${tag}/${name}`,
      digest: digest === undefined ? `sha256:${DIGEST_A}` : digest,
    })),
  };
}

/** A fake GitHub API: each URL answers with its fixture, anything else is a test failure. */
function api(answers: Record<string, unknown>) {
  return vi.fn(async (url: string) => {
    if (!(url in answers)) throw new Error(`unexpected request: ${url}`);
    return answers[url];
  });
}

describe('a BAAM-listed extension follows its registry row', () => {
  it('takes the row tag, asset and the digest GitHub publishes for it, never releases/latest', async () => {
    const fetchJson = api({
      'https://api.github.com/repos/Broccolito/gptr/releases/tags/bridge-v1.4.0': release(
        'bridge-v1.4.0',
        [
          { name: 'gptr-1.4.0.brxt', digest: `sha256:${DIGEST_B}` },
          { name: 'gptr.brxt', digest: `sha256:${DIGEST_B}` },
          { name: 'gptragent.brxt', digest: `sha256:${DIGEST_A.toUpperCase()}` },
        ],
        { prerelease: true }
      ),
    });

    const decision = await chooseExtensionRelease({
      folderName: 'gptr',
      manifest: GPTRAGENT_INSTALLED,
      registry: registryOf(gptragentRow()),
      fetchJson,
    });

    expect(decision).toEqual({
      kind: 'update',
      release: {
        from: 'registry',
        version: '1.4.0',
        tag: 'bridge-v1.4.0',
        assetName: 'gptragent.brxt',
        downloadUrl:
          'https://github.com/Broccolito/gptr/releases/download/bridge-v1.4.0/gptragent.brxt',
        sha256: DIGEST_A,
        prerelease: true,
      },
    });
    // A never-latest prerelease is reachable only by its tag: latest was never asked.
    expect(fetchJson).toHaveBeenCalledTimes(1);
  });

  it('matches the row under the new id too, once the catalog moves to it', async () => {
    const fetchJson = api({
      'https://api.github.com/repos/Broccolito/gptr/releases/tags/bridge-v1.4.0': release(
        'bridge-v1.4.0',
        [{ name: 'gptr.brxt' }]
      ),
    });
    const decision = await chooseExtensionRelease({
      folderName: 'gptr',
      manifest: GPTRAGENT_INSTALLED,
      registry: registryOf(
        gptragentRow({
          id: 'gptr',
          extension_name: 'gptr',
          filename: 'gptr.brxt',
          download: 'https://github.com/Broccolito/gptr/releases/download/bridge-v1.4.0/gptr.brxt',
        })
      ),
      fetchJson,
    });
    expect(decision).toMatchObject({ kind: 'update', release: { assetName: 'gptr.brxt' } });
  });

  it('reads a row without extension_name by its id, and an encoded tag as the tag it names', async () => {
    const fetchJson = api({
      'https://api.github.com/repos/Broccolito/folklore-mcp/releases/tags/folklore-biorouter-v2.1.0%2Bbiorouter.1':
        release('folklore-biorouter-v2.1.0+biorouter.1', [{ name: 'folklore.brxt' }]),
    });
    const decision = await chooseExtensionRelease({
      folderName: 'folklore',
      manifest: { name: 'folklore', version: '2.0.0', entry_point: 'folklore' },
      registry: registryOf({
        id: 'folklore',
        version: 'v2.1.0+biorouter.1',
        download:
          'https://github.com/Broccolito/folklore-mcp/releases/download/folklore-biorouter-v2.1.0%2Bbiorouter.1/folklore.brxt',
        filename: 'folklore.brxt',
      }),
      fetchJson,
    });
    expect(decision).toMatchObject({
      kind: 'update',
      release: { tag: 'folklore-biorouter-v2.1.0+biorouter.1', version: '2.1.0+biorouter.1' },
    });
  });

  it('asks the network nothing when the row is not newer than the install', async () => {
    const fetchJson = api({});
    const decision = await chooseExtensionRelease({
      folderName: 'gptr',
      manifest: { ...GPTRAGENT_INSTALLED, version: '1.4.0' },
      registry: registryOf(gptragentRow()),
      fetchJson,
    });
    expect(decision).toEqual({ kind: 'up-to-date', version: '1.4.0' });
    expect(fetchJson).not.toHaveBeenCalled();
  });

  it('refuses when two rows name the same extension', async () => {
    const fetchJson = api({});
    const decision = await chooseExtensionRelease({
      folderName: 'gptr',
      manifest: GPTRAGENT_INSTALLED,
      registry: registryOf(gptragentRow(), gptragentRow({ id: 'gptr', extension_name: 'gptr' })),
      fetchJson,
    });
    expect(decision.kind).toBe('refuse');
    expect(fetchJson).not.toHaveBeenCalled();
  });

  it.each([
    [
      'a host other than github.com',
      'https://example.com/o/r/releases/download/v1.4.0/gptragent.brxt',
    ],
    ['plain http', 'http://github.com/o/r/releases/download/v1.4.0/gptragent.brxt'],
    ['a query', 'https://github.com/o/r/releases/download/v1.4.0/gptragent.brxt?x=1'],
    [
      'a file name other than the row',
      'https://github.com/o/r/releases/download/v1.4.0/other.brxt',
    ],
    [
      'a path that is not a release download',
      'https://github.com/o/r/archive/v1.4.0/gptragent.brxt',
    ],
  ])('refuses a row whose download has %s', async (_label, download) => {
    const fetchJson = api({});
    const decision = await chooseExtensionRelease({
      folderName: 'gptr',
      manifest: GPTRAGENT_INSTALLED,
      registry: registryOf(gptragentRow({ download })),
      fetchJson,
    });
    expect(decision.kind).toBe('refuse');
    expect(fetchJson).not.toHaveBeenCalled();
  });

  it.each([
    ['no digest', null],
    ['a digest of another algorithm', `sha512:${'c'.repeat(128)}`],
    ['a truncated digest', `sha256:${'a'.repeat(63)}`],
  ])('refuses a release asset with %s', async (_label, digest) => {
    const fetchJson = api({
      'https://api.github.com/repos/Broccolito/gptr/releases/tags/bridge-v1.4.0': release(
        'bridge-v1.4.0',
        [{ name: 'gptragent.brxt', digest }]
      ),
    });
    const decision = await chooseExtensionRelease({
      folderName: 'gptr',
      manifest: GPTRAGENT_INSTALLED,
      registry: registryOf(gptragentRow()),
      fetchJson,
    });
    expect(decision.kind).toBe('refuse');
  });

  it('refuses when the tagged release does not carry the row asset', async () => {
    const fetchJson = api({
      'https://api.github.com/repos/Broccolito/gptr/releases/tags/bridge-v1.4.0': release(
        'bridge-v1.4.0',
        [{ name: 'gptr.brxt' }]
      ),
    });
    const decision = await chooseExtensionRelease({
      folderName: 'gptr',
      manifest: GPTRAGENT_INSTALLED,
      registry: registryOf(gptragentRow()),
      fetchJson,
    });
    expect(decision.kind).toBe('refuse');
  });

  it('refuses when GitHub answers for a different tag', async () => {
    const fetchJson = api({
      'https://api.github.com/repos/Broccolito/gptr/releases/tags/bridge-v1.4.0': release(
        'v9.9.9',
        [{ name: 'gptragent.brxt' }]
      ),
    });
    const decision = await chooseExtensionRelease({
      folderName: 'gptr',
      manifest: GPTRAGENT_INSTALLED,
      registry: registryOf(gptragentRow()),
      fetchJson,
    });
    expect(decision.kind).toBe('refuse');
  });
});

describe('an extension BAAM does not list takes <name>.brxt from releases/latest', () => {
  it('picks gptragent.brxt when GitHub lists gptr.brxt first (case-insensitive name order)', async () => {
    // The order GitHub really returns: `-` and `.` sort before `a`, so every
    // gptr-named bundle comes before the gptragent ones.
    const latest = release('v1.4.0', [
      { name: 'gptr-1.4.0.brxt', digest: `sha256:${DIGEST_B}` },
      { name: 'gptr.brxt', digest: `sha256:${DIGEST_B}` },
      { name: 'gptragent-1.4.0.brxt', digest: `sha256:${DIGEST_B}` },
      { name: 'gptragent.brxt', digest: `sha256:${DIGEST_A}` },
      { name: 'SHA256SUMS', digest: `sha256:${DIGEST_B}` },
    ]);
    const fetchJson = api({
      'https://api.github.com/repos/Broccolito/gptr/releases/latest': latest,
    });

    const decision = await chooseExtensionRelease({
      folderName: 'gptr',
      manifest: GPTRAGENT_INSTALLED,
      registry: registryOf(),
      fetchJson,
    });

    expect(decision).toEqual({
      kind: 'update',
      release: {
        from: 'latest',
        version: '1.4.0',
        tag: 'v1.4.0',
        assetName: 'gptragent.brxt',
        downloadUrl: 'https://github.com/owner/repo/releases/download/v1.4.0/gptragent.brxt',
        sha256: DIGEST_A,
        prerelease: false,
      },
    });
  });

  it('matches the asset name without regard to case, as GitHub does', async () => {
    const fetchJson = api({
      'https://api.github.com/repos/BaranziniLab/CDWAgent/releases/latest': release('v0.8.0', [
        { name: 'CDWAgent.brxt' },
      ]),
    });
    const decision = await chooseExtensionRelease({
      folderName: 'CDWAgent',
      manifest: {
        name: 'CDWAgent',
        version: '0.7.0',
        entry_point: 'cdwagent',
        repository: 'https://github.com/BaranziniLab/CDWAgent',
      },
      registry: null,
      fetchJson,
    });
    expect(decision).toMatchObject({ kind: 'update', release: { assetName: 'CDWAgent.brxt' } });
  });

  it('refuses a release that has only other .brxt files', async () => {
    const fetchJson = api({
      'https://api.github.com/repos/Broccolito/gptr/releases/latest': release('v1.4.0', [
        { name: 'gptr.brxt' },
        { name: 'gptragent-1.4.0.brxt' },
      ]),
    });
    const decision = await chooseExtensionRelease({
      folderName: 'gptr',
      manifest: GPTRAGENT_INSTALLED,
      registry: registryOf(),
      fetchJson,
    });
    expect(decision.kind).toBe('refuse');
  });

  it('refuses the asset when GitHub published no digest for it', async () => {
    const fetchJson = api({
      'https://api.github.com/repos/Broccolito/gptr/releases/latest': release('v1.4.0', [
        { name: 'gptragent.brxt', digest: null },
      ]),
    });
    const decision = await chooseExtensionRelease({
      folderName: 'gptr',
      manifest: GPTRAGENT_INSTALLED,
      registry: registryOf(),
      fetchJson,
    });
    expect(decision.kind).toBe('refuse');
  });

  it('refuses a download address that is not https', async () => {
    const fetchJson = api({
      'https://api.github.com/repos/Broccolito/gptr/releases/latest': release('v1.4.0', [
        { name: 'gptragent.brxt', url: 'http://github.com/x/gptragent.brxt' },
      ]),
    });
    const decision = await chooseExtensionRelease({
      folderName: 'gptr',
      manifest: GPTRAGENT_INSTALLED,
      registry: registryOf(),
      fetchJson,
    });
    expect(decision.kind).toBe('refuse');
  });

  it('is up to date when latest is not newer, before looking at any asset', async () => {
    const fetchJson = api({
      'https://api.github.com/repos/Broccolito/gptr/releases/latest': release('v1.3.0', []),
    });
    const decision = await chooseExtensionRelease({
      folderName: 'gptr',
      manifest: GPTRAGENT_INSTALLED,
      registry: registryOf(),
      fetchJson,
    });
    expect(decision).toEqual({ kind: 'up-to-date', version: '1.3.0' });
  });

  it('skips an extension whose manifest names no GitHub repository', async () => {
    const fetchJson = api({});
    for (const repository of [
      undefined,
      'https://git.example.invalid/o/r',
      'https://evilgithub.com/o/r',
    ]) {
      const decision = await chooseExtensionRelease({
        folderName: 'mytool',
        manifest: { name: 'mytool', version: '1.0.0', entry_point: 'main.py', repository },
        registry: registryOf(),
        fetchJson,
      });
      expect(decision.kind).toBe('skip');
    }
    expect(fetchJson).not.toHaveBeenCalled();
  });
});

describe('a folder under an old id waits for the daemon', () => {
  it('skips extensions/gptragent without asking anything', async () => {
    const fetchJson = api({});
    const decision = await chooseExtensionRelease({
      folderName: 'gptragent',
      manifest: GPTRAGENT_INSTALLED,
      registry: registryOf(gptragentRow()),
      fetchJson,
    });
    expect(decision.kind).toBe('skip');
    expect(fetchJson).not.toHaveBeenCalled();
  });
});

describe('the bundle has to be the installed extension at the release version', () => {
  const bundle = (overrides: Record<string, unknown> = {}) => ({
    name: 'gptragent',
    display_name: 'gptr Agent',
    version: '1.4.0',
    entry_point: 'gptragent',
    ...overrides,
  });

  it('accepts the same extension under either of its ids', () => {
    expect(checkBundleManifest(GPTRAGENT_INSTALLED, bundle(), '1.4.0')).toBeNull();
    expect(checkBundleManifest(GPTRAGENT_INSTALLED, bundle({ name: 'gptr' }), '1.4.0')).toBeNull();
    expect(
      checkBundleManifest(GPTRAGENT_INSTALLED, bundle({ version: 'v1.4.0' }), '1.4.0')
    ).toBeNull();
  });

  it('refuses a bundle for another extension', () => {
    expect(
      checkBundleManifest(GPTRAGENT_INSTALLED, bundle({ name: 'spokeagent' }), '1.4.0')
    ).toMatch(/spokeagent/);
  });

  it('refuses a bundle whose entry point the installed configuration does not run', () => {
    expect(
      checkBundleManifest(
        GPTRAGENT_INSTALLED,
        bundle({ name: 'gptr', entry_point: 'gptr' }),
        '1.4.0'
      )
    ).toMatch(/gptr/);
  });

  it('refuses a bundle whose version is not the release version', () => {
    expect(checkBundleManifest(GPTRAGENT_INSTALLED, bundle({ version: '1.4.1' }), '1.4.0')).toMatch(
      /1\.4\.1/
    );
  });

  it('refuses a bundle with no readable manifest', () => {
    expect(checkBundleManifest(GPTRAGENT_INSTALLED, null, '1.4.0')).not.toBeNull();
    expect(checkBundleManifest(GPTRAGENT_INSTALLED, { name: 'gptragent' }, '1.4.0')).not.toBeNull();
  });
});

describe('the small parsers', () => {
  it('reads only a sha256 digest of 64 hex digits', () => {
    expect(sha256FromDigest(`sha256:${DIGEST_A.toUpperCase()}`)).toBe(DIGEST_A);
    expect(sha256FromDigest(DIGEST_A)).toBeNull();
    expect(sha256FromDigest(null)).toBeNull();
    expect(sha256FromDigest(`sha256:${DIGEST_A}0`)).toBeNull();
  });

  it('reads a GitHub release download address and nothing else', () => {
    expect(
      parseReleaseDownload(
        'https://github.com/Broccolito/gptr/releases/download/bridge-v1.4.0/gptr.brxt',
        'gptr.brxt'
      )
    ).toEqual({ owner: 'Broccolito', repo: 'gptr', tag: 'bridge-v1.4.0' });
    expect(
      parseReleaseDownload(
        'https://github.com/Broccolito/gptr/releases/download/bridge-v1.4.0/gptr.brxt#x',
        'gptr.brxt'
      )
    ).toBeNull();
    expect(
      parseReleaseDownload('https://github.com/../x/releases/download/v1/gptr.brxt', 'gptr.brxt')
    ).toBeNull();
  });

  it('finds rows by every id of the installed extension', () => {
    const rows = registryRowsFor(
      registryOf(gptragentRow(), { id: 'spokeagent', extension_name: 'spokeagent' }, null, 'x'),
      'gptr'
    );
    expect(rows.map((row) => row.id)).toEqual(['gptragent']);
    expect(registryRowsFor({ extensions: 'nope' }, 'gptr')).toEqual([]);
    expect(registryRowsFor(null, 'gptr')).toEqual([]);
  });
});
