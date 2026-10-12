// @vitest-environment node

/**
 * The GitHub fallback compares the latest release tag with the running version. In a development
 * launch that hands Electron `.vite/build/main.js` directly, `app.getVersion()` is Electron's own
 * version (39.8.10), and every Biorouter release compared older than it: the check answered "up to
 * date" forever. The comparison must be made against Biorouter's version.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

const ELECTRON_VERSION = '39.8.10';

vi.mock('electron', () => ({ app: { getVersion: () => ELECTRON_VERSION } }));
vi.mock('./logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() },
}));

import { GitHubUpdater } from './githubUpdater';
import { BUNDLED_BIOROUTER_VERSION } from './biorouterVersion';

/** The next patch release after the version this bundle was built at. */
function nextPatch(version: string): string {
  const [major, minor, patch] = version.split('.').map((part) => parseInt(part, 10));
  return `${major}.${minor}.${patch + 1}`;
}

function releaseResponse(tag: string): Response {
  const v = tag.replace(/^v/, '');
  const names = [
    `Biorouter-${v}-arm64.dmg`,
    `Biorouter-${v}-x64.dmg`,
    `Biorouter-Setup-${v}.exe`,
    `Biorouter-win32-x64-${v}.zip`,
    `biorouter_${v}_amd64.deb`,
  ];
  const body = {
    tag_name: tag,
    name: `Biorouter ${tag}`,
    published_at: '2026-10-12T00:00:00Z',
    html_url: `https://github.com/BaranziniLab/biorouter/releases/tag/${tag}`,
    assets: names.map((name) => ({
      name,
      browser_download_url: `https://example.invalid/${name}`,
      size: 1,
    })),
  };
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  });
}

let hadElectronVersion: boolean;

beforeEach(() => {
  // What the main process sees: Electron's version in process.versions, and the same string from
  // app.getVersion() because no package.json was loaded.
  hadElectronVersion = 'electron' in process.versions;
  Object.defineProperty(process.versions, 'electron', {
    value: ELECTRON_VERSION,
    configurable: true,
  });
});

afterEach(() => {
  if (!hadElectronVersion) delete (process.versions as Record<string, unknown>).electron;
  vi.unstubAllGlobals();
});

describe('GitHubUpdater compares Biorouter versions', () => {
  it('offers the next Biorouter release in a launch where Electron reports its own version', async () => {
    const latest = nextPatch(BUNDLED_BIOROUTER_VERSION);
    const fetchMock = vi.fn().mockResolvedValue(releaseResponse(`v${latest}`));
    vi.stubGlobal('fetch', fetchMock);

    const result = await new GitHubUpdater().checkForUpdates();

    expect(result.error).toBeUndefined();
    expect(result.updateAvailable).toBe(true);
    expect(result.latestVersion).toBe(latest);
    const headers = (fetchMock.mock.calls[0][1] as RequestInit).headers as Record<string, string>;
    expect(headers['User-Agent']).toBe(`Biorouter-Desktop/${BUNDLED_BIOROUTER_VERSION}`);
  });

  it('answers up to date for the release this bundle was built from', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(releaseResponse(`v${BUNDLED_BIOROUTER_VERSION}`))
    );

    const result = await new GitHubUpdater().checkForUpdates();

    expect(result.updateAvailable).toBe(false);
    expect(result.latestVersion).toBe(BUNDLED_BIOROUTER_VERSION);
  });
});
