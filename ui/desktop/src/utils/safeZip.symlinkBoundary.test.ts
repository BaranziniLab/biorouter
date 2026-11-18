// @vitest-environment node

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import AdmZip from 'adm-zip';
import fs from 'node:fs';
import path from 'node:path';
import { runProbe } from './dependencyChecker';
import { safeExtractZip } from './safeZip';

const fixturePaths = vi.hoisted(() => ({ extensions: '', temporary: '' }));
const events: Array<Record<string, unknown>> = [];

vi.mock('electron', () => ({
  BrowserWindow: {
    getAllWindows: () => [
      {
        isDestroyed: () => false,
        webContents: {
          send: (_channel: string, event: Record<string, unknown>) => events.push(event),
        },
      },
    ],
  },
}));

vi.mock('./logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() },
}));

vi.mock('./dependencyChecker', () => ({ runProbe: vi.fn() }));

vi.mock('./biorouterPaths', () => ({
  biorouterExtensionsDir: () => {
    if (!fixturePaths.extensions) throw new Error('synthetic extension root is not initialized');
    return fixturePaths.extensions;
  },
}));

vi.mock('os', async (importOriginal) => {
  const actual = await importOriginal<typeof import('os')>();
  const tmpdir = () => {
    if (!fixturePaths.temporary) throw new Error('synthetic download root is not initialized');
    return fixturePaths.temporary;
  };
  return { ...actual, default: { ...actual, tmpdir }, tmpdir };
});

const TEMP_BASE = process.env.TMPDIR || process.env.TEMP || '/tmp';
const OUTSIDE_ORIGINAL = 'synthetic outside original\n';
const ARCHIVE_REPLACEMENT = 'synthetic archive replacement\n';
let qaRoot: string;

beforeEach(() => {
  qaRoot = fs.realpathSync(fs.mkdtempSync(path.join(TEMP_BASE, 'biorouter-zip-boundary-')));
  fixturePaths.extensions = path.join(qaRoot, 'config', 'extensions');
  fixturePaths.temporary = path.join(qaRoot, 'downloads');
  fs.mkdirSync(fixturePaths.extensions, { recursive: true });
  fs.mkdirSync(fixturePaths.temporary);
  events.length = 0;
  vi.mocked(runProbe).mockReset();
});

afterEach(() => {
  vi.unstubAllGlobals();
  fs.rmSync(qaRoot, { recursive: true, force: true });
  fixturePaths.extensions = '';
  fixturePaths.temporary = '';
});

type LinkKind = 'file' | 'ancestor directory';

function linkedPackage(kind: LinkKind) {
  const name = 'synthetic-boundary-extension';
  const installDir = path.join(fixturePaths.extensions, name);
  const outsideDir = path.join(qaRoot, 'outside-installed-tree');
  const outsideTarget = path.join(outsideDir, 'sentinel.txt');
  fs.mkdirSync(installDir);
  fs.mkdirSync(outsideDir);
  fs.writeFileSync(outsideTarget, OUTSIDE_ORIGINAL);
  const linkedPath = path.join(installDir, kind === 'file' ? 'linked.txt' : 'assets');
  const linkTarget = kind === 'file' ? outsideTarget : outsideDir;
  const linkType = kind === 'file' ? 'file' : process.platform === 'win32' ? 'junction' : 'dir';
  fs.symlinkSync(linkTarget, linkedPath, linkType);
  const member = kind === 'file' ? 'linked.txt' : 'assets/sentinel.txt';
  const manifest = {
    name,
    display_name: name,
    description: 'synthetic boundary fixture',
    version: '1.0.0',
    entry_point: 'main.py',
    repository: 'https://github.com/synthetic-fixtures/boundary-extension',
  };
  fs.writeFileSync(path.join(installDir, 'manifest.json'), JSON.stringify(manifest));
  fs.writeFileSync(path.join(installDir, 'regular.txt'), 'installed original\n');
  return { name, installDir, outsideTarget, linkedPath, linkTarget, member, manifest };
}

describe('ZIP extraction filesystem boundary', () => {
  it.each(['file', 'ancestor directory'] as const)(
    'refuses an existing %s link without changing its outside referent',
    (kind) => {
      const fixture = linkedPackage(kind);
      const zip = new AdmZip();
      zip.addFile(fixture.member, Buffer.from(ARCHIVE_REPLACEMENT));
      let failure: unknown;

      try {
        safeExtractZip(zip, fixture.installDir);
      } catch (error) {
        failure = error;
      }

      expect(fs.readFileSync(fixture.outsideTarget, 'utf8')).toBe(OUTSIDE_ORIGINAL);
      expect(failure).toBeInstanceOf(Error);
      expect(fs.lstatSync(fixture.linkedPath).isSymbolicLink()).toBe(true);
      expect(fs.realpathSync(fixture.linkedPath)).toBe(fs.realpathSync(fixture.linkTarget));
    }
  );

  it('still writes legitimate nested files inside the extraction directory', () => {
    const installDir = path.join(fixturePaths.extensions, 'regular-package');
    const outsideTarget = path.join(qaRoot, 'unrelated.txt');
    fs.writeFileSync(outsideTarget, OUTSIDE_ORIGINAL);
    const zip = new AdmZip();
    zip.addFile('src/nested/main.py', Buffer.from('synthetic regular file\n'));

    safeExtractZip(zip, installDir);

    expect(fs.readFileSync(path.join(installDir, 'src', 'nested', 'main.py'), 'utf8')).toBe(
      'synthetic regular file\n'
    );
    expect(fs.readFileSync(outsideTarget, 'utf8')).toBe(OUTSIDE_ORIGINAL);
  });

  it('refuses a dangling file link rather than creating its outside referent', () => {
    const fixture = linkedPackage('file');
    fs.unlinkSync(fixture.outsideTarget);
    const zip = new AdmZip();
    zip.addFile(fixture.member, Buffer.from(ARCHIVE_REPLACEMENT));

    expect(() => safeExtractZip(zip, fixture.installDir)).toThrow();

    expect(fs.existsSync(fixture.outsideTarget)).toBe(false);
    expect(fs.lstatSync(fixture.linkedPath).isSymbolicLink()).toBe(true);
  });

  it('allows an existing link whose referent stays inside the extraction directory', () => {
    const installDir = path.join(fixturePaths.extensions, 'internal-link-package');
    fs.mkdirSync(installDir);
    const target = path.join(installDir, 'target.txt');
    const link = path.join(installDir, 'linked.txt');
    fs.writeFileSync(target, 'installed original\n');
    fs.symlinkSync(target, link, 'file');
    const zip = new AdmZip();
    zip.addFile('linked.txt', Buffer.from(ARCHIVE_REPLACEMENT));

    safeExtractZip(zip, installDir);

    expect(fs.readFileSync(target, 'utf8')).toBe(ARCHIVE_REPLACEMENT);
    expect(fs.lstatSync(link).isSymbolicLink()).toBe(true);
  });
});

describe('extension updater with pre-existing links', () => {
  it.each(['file', 'ancestor directory'] as const)(
    'rejects the %s escape before uv and preserves the external bytes through rollback',
    async (kind) => {
      const fixture = linkedPackage(kind);
      const zip = new AdmZip();
      zip.addFile(
        'manifest.json',
        Buffer.from(JSON.stringify({ ...fixture.manifest, version: '2.0.0' }))
      );
      zip.addFile('regular.txt', Buffer.from('replacement regular file\n'));
      zip.addFile('new.txt', Buffer.from('new update file\n'));
      zip.addFile(fixture.member, Buffer.from(ARCHIVE_REPLACEMENT));
      const archive = zip.toBuffer();
      const releaseUrl =
        'https://api.github.com/repos/synthetic-fixtures/boundary-extension/releases/latest';
      const downloadUrl = 'https://downloads.example.invalid/boundary-extension/2.0.0.brxt';
      vi.stubGlobal(
        'fetch',
        vi.fn(async (input: string | URL | Request) => {
          const url = String(input);
          if (url === releaseUrl) {
            return new Response(
              JSON.stringify({
                tag_name: 'v2.0.0',
                assets: [
                  {
                    name: 'boundary-extension.brxt',
                    browser_download_url: downloadUrl,
                    size: archive.length,
                  },
                ],
              }),
              { status: 200 }
            );
          }
          if (url !== downloadUrl) throw new Error(`unexpected fixture request: ${url}`);
          return new Response(new Uint8Array(archive), { status: 200 });
        })
      );
      const observedAtProbe: string[] = [];
      vi.mocked(runProbe).mockImplementation(async () => {
        observedAtProbe.push(fs.readFileSync(fixture.outsideTarget, 'utf8'));
        return {
          ok: false,
          code: 19,
          stdout: '',
          stderr: 'synthetic sync failure; no process was spawned',
          timedOut: false,
          error: null,
        };
      });
      vi.resetModules();
      const { runExtensionUpdateCheck } = await import('./extensionUpdater');

      await runExtensionUpdateCheck();

      expect(fs.readFileSync(fixture.outsideTarget, 'utf8')).toBe(OUTSIDE_ORIGINAL);
      expect(observedAtProbe).toEqual([]);
      expect(runProbe).not.toHaveBeenCalled();
      expect(
        JSON.parse(fs.readFileSync(path.join(fixture.installDir, 'manifest.json'), 'utf8')).version
      ).toBe('1.0.0');
      expect(fs.readFileSync(path.join(fixture.installDir, 'regular.txt'), 'utf8')).toBe(
        'installed original\n'
      );
      expect(fs.existsSync(path.join(fixture.installDir, 'new.txt'))).toBe(false);
      expect(fs.lstatSync(fixture.linkedPath).isSymbolicLink()).toBe(true);
      expect(fs.realpathSync(fixture.linkedPath)).toBe(fs.realpathSync(fixture.linkTarget));
      expect(events).toContainEqual(
        expect.objectContaining({ type: 'update-error', ext: fixture.name })
      );
      expect(events.filter((event) => event.type === 'update-done')).toEqual([]);
      expect(events[events.length - 1]).toEqual({ type: 'all-done', updatedCount: 0 });
      expect(fs.readdirSync(fixturePaths.extensions)).toEqual([fixture.name]);
      expect(fs.readdirSync(fixturePaths.temporary)).toEqual([]);
    }
  );
});
