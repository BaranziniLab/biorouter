// @vitest-environment node
import AdmZip from 'adm-zip';
import { promises as fs } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  exportAppToFolder,
  extractExportZip,
  resetExportRetriesForTests,
  type AppExportDeps,
} from './appExport';
import { EXPORT_REPORT_HEADER, type AppExportRequest } from './appExportRequest';

const APP = 'cohort-explorer';
const PNG = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe]);

/** A zip shaped like the daemon's: everything under `<id>/`, modes in the external attributes. */
function daemonZip(files: Record<string, [Buffer | string, number]>, id = APP): Buffer {
  const zip = new AdmZip();
  for (const [rel, [content, mode]] of Object.entries(files)) {
    zip.addFile(
      `${id}/${rel}`,
      Buffer.isBuffer(content) ? content : Buffer.from(content),
      '',
      mode
    );
  }
  return zip.toBuffer();
}

const standardZip = () =>
  daemonZip({
    'index.html': ['<html><body>hi</body></html>', 0o644],
    'manifest.json': [JSON.stringify({ id: APP }), 0o644],
    'run.command': ['#!/usr/bin/env bash\n', 0o755],
    'run.sh': ['#!/usr/bin/env bash\n', 0o755],
    'biorouter-launch.sh': ['#!/usr/bin/env bash\n', 0o755],
    'assets/logo.png': [PNG, 0o644],
  });

const report = (value: unknown) => Buffer.from(JSON.stringify(value)).toString('base64url');

function zipResponse(zip: Buffer, summary: unknown = { mode: 'launcher', files: 6, notes: [] }) {
  return new Response(new Uint8Array(zip), {
    status: 200,
    headers: { 'Content-Type': 'application/zip', [EXPORT_REPORT_HEADER]: report(summary) },
  });
}

const launcher: AppExportRequest = {
  mode: 'launcher',
  bundleDaemon: false,
  allowLintErrors: false,
};

let root: string;
let picked: string;

beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), 'app-export-'));
  picked = path.join(root, 'Desktop');
  await fs.mkdir(picked);
  resetExportRetriesForTests();
});

afterEach(async () => {
  await fs.rm(root, { recursive: true, force: true });
});

function deps(overrides: Partial<AppExportDeps> = {}) {
  const events: string[] = [];
  const fetch = vi.fn(async (_url: string, init?: RequestInit) => {
    events.push(`fetch ${init?.method ?? 'GET'}`);
    return zipResponse(standardZip());
  });
  const base: AppExportDeps = {
    baseUrl: 'http://127.0.0.1:51234/',
    secretKey: 'desktop-secret',
    userActionKey: 'desktop-proof',
    chooseDirectory: vi.fn(async () => {
      events.push('pick');
      return picked;
    }),
    confirmReplace: vi.fn(async () => {
      events.push('confirm');
      return true;
    }),
    trash: vi.fn(async (target: string) => {
      events.push(`trash ${path.basename(target)}`);
      await fs.rm(target, { recursive: true, force: true });
    }),
    fetch: fetch as unknown as typeof globalThis.fetch,
    ...overrides,
  };
  return { deps: base, events, fetch };
}

const modeOf = async (file: string) => (await fs.stat(file)).mode & 0o777;

describe('exportAppToFolder', () => {
  it('asks for the folder before anything is asked of the daemon, and a cancel sends nothing', async () => {
    const {
      deps: d,
      events,
      fetch,
    } = deps({
      chooseDirectory: vi.fn(async () => undefined),
    });
    expect(await exportAppToFolder(APP, launcher, d)).toEqual({ status: 'canceled' });
    expect(fetch).not.toHaveBeenCalled();
    expect(events).toEqual([]);

    const ok = deps();
    const result = await exportAppToFolder(APP, launcher, ok.deps);
    expect(result.status).toBe('exported');
    expect(ok.events).toEqual(['pick', 'fetch POST']);
  });

  it('posts the request with the secret and the proof, and every include list spelled out', async () => {
    const { deps: d, fetch } = deps();
    await exportAppToFolder(
      APP,
      {
        mode: 'full',
        bundleDaemon: true,
        allowLintErrors: false,
        include: { knowledge_bases: [], skills: ['ggplot'], extensions: [] },
      },
      d
    );
    const [url, init] = fetch.mock.calls[0];
    expect(url).toBe(`http://127.0.0.1:51234/apps/${APP}/export`);
    expect(init?.method).toBe('POST');
    expect(init?.headers).toMatchObject({
      'X-Secret-Key': 'desktop-secret',
      'X-User-Action': 'desktop-proof',
      'Content-Type': 'application/json',
    });
    expect(JSON.parse(String(init?.body))).toEqual({
      mode: 'full',
      include: { knowledge_bases: [], skills: ['ggplot'], extensions: [] },
      bundle_daemon: 'current',
      allow_lint_errors: false,
    });
  });

  it('unpacks the zip into <folder>/<app id>, byte for byte, launchers executable', async () => {
    const { deps: d } = deps();
    const result = await exportAppToFolder(APP, launcher, d);
    const target = path.join(picked, APP);
    expect(result).toMatchObject({ status: 'exported', target, legacy: false });
    expect(await fs.readFile(path.join(target, 'assets/logo.png'))).toEqual(PNG);
    if (process.platform !== 'win32') {
      for (const launcher of ['run.command', 'run.sh', 'biorouter-launch.sh']) {
        expect(await modeOf(path.join(target, launcher))).toBe(0o755);
      }
      expect(await modeOf(path.join(target, 'index.html'))).toBe(0o644);
    }
    // Nothing half-written is left beside it.
    expect(await fs.readdir(picked)).toEqual([APP]);
  });

  it('restores executable modes for bundled daemons and native compilers without zip modes', async () => {
    const paths = [
      'payload/bin/biorouterd',
      'payload/bin/biorouterd.exe',
      'payload/bin/esbuild',
      'payload/bin/esbuild.exe',
    ];
    const files: Record<string, [Buffer | string, number]> = {
      'assets/esbuild': ['authored asset', 0o644],
    };
    for (const executable of paths) files[executable] = [PNG, 0o644];
    const target = path.join(picked, APP);
    extractExportZip(daemonZip(files), APP, target);
    for (const executable of paths) {
      expect(await fs.readFile(path.join(target, executable))).toEqual(PNG);
      if (process.platform !== 'win32') {
        expect(await modeOf(path.join(target, executable))).toBe(0o755);
      }
    }
    if (process.platform !== 'win32') {
      expect(await modeOf(path.join(target, 'assets/esbuild'))).toBe(0o644);
    }
  });

  it('replaces a folder with something in it only on a yes, and moves the old one to the Trash', async () => {
    const target = path.join(picked, APP);
    await fs.mkdir(target);
    await fs.writeFile(path.join(target, 'thesis.tex'), 'precious');

    const declined = deps({ confirmReplace: vi.fn(async () => false) });
    expect(await exportAppToFolder(APP, launcher, declined.deps)).toEqual({ status: 'canceled' });
    expect(declined.fetch).not.toHaveBeenCalled();
    expect(await fs.readFile(path.join(target, 'thesis.tex'), 'utf8')).toBe('precious');

    const accepted = deps();
    const result = await exportAppToFolder(APP, launcher, accepted.deps);
    expect(result.status).toBe('exported');
    expect(accepted.events[0]).toBe('pick');
    expect(accepted.events[1]).toBe('confirm');
    expect(accepted.events).toContain('fetch POST');
    expect(accepted.events.some((event) => event.startsWith(`trash .${APP}.replaced-`))).toBe(true);
    await expect(fs.stat(path.join(target, 'thesis.tex'))).rejects.toThrow();
    expect(await fs.readdir(picked)).toEqual([APP]);
  });

  it('writes into an empty folder of that name without asking', async () => {
    await fs.mkdir(path.join(picked, APP));
    const { deps: d, events } = deps();
    expect((await exportAppToFolder(APP, launcher, d)).status).toBe('exported');
    expect(events).not.toContain('confirm');
  });

  it('answers harness errors with their findings, and "export anyway" reuses the folder', async () => {
    let calls = 0;
    const fetch = vi.fn(async (_url: string, init?: RequestInit) => {
      calls += 1;
      if (calls === 1) {
        return new Response(
          JSON.stringify({ error: 'harness ERRORs block export', findings: ['#missing'] }),
          { status: 422, headers: { 'Content-Type': 'application/json' } }
        );
      }
      expect(JSON.parse(String(init?.body)).allow_lint_errors).toBe(true);
      return zipResponse(standardZip());
    });
    const { deps: d } = deps({ fetch: fetch as unknown as typeof globalThis.fetch });

    const first = await exportAppToFolder(APP, launcher, d);
    expect(first).toMatchObject({
      status: 'lint',
      message: 'harness ERRORs block export',
      findings: ['#missing'],
    });
    expect(await fs.readdir(picked)).toEqual([]);
    const retry = (first as { retry: string }).retry;

    const second = await exportAppToFolder(APP, { ...launcher, allowLintErrors: true, retry }, d);
    expect(second.status).toBe('exported');
    expect(d.chooseDirectory).toHaveBeenCalledTimes(1);

    // A token works once, and only for its own app.
    await exportAppToFolder('other-app', { ...launcher, allowLintErrors: true, retry }, d);
    expect(d.chooseDirectory).toHaveBeenCalledTimes(2);
  });

  it('falls back to the legacy GET on an older daemon, launchers executable, and says so', async () => {
    const fetch = vi.fn(async (_url: string, init?: RequestInit) => {
      if (init?.method === 'POST') return new Response('Method Not Allowed', { status: 405 });
      return new Response(
        JSON.stringify({
          id: APP,
          files: {
            'index.html': '<html></html>',
            'run.command': '#!/usr/bin/env bash\n',
            'run.sh': '#!/usr/bin/env bash\n',
            'biorouter-launch.sh': '#!/usr/bin/env bash\n',
          },
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      );
    });
    const { deps: d } = deps({ fetch: fetch as unknown as typeof globalThis.fetch });
    const result = await exportAppToFolder(
      APP,
      {
        mode: 'full',
        bundleDaemon: false,
        allowLintErrors: false,
        include: { knowledge_bases: [], skills: [], extensions: [] },
      },
      d
    );
    expect(result).toMatchObject({ status: 'exported', legacy: true });
    if (result.status !== 'exported') return;
    expect(result.summary.files).toBe(4);
    expect(result.summary.notes.join(' ')).toMatch(/older/);
    expect(result.summary.notes.join(' ')).toMatch(/Update Biorouter/);
    if (process.platform !== 'win32') {
      expect(await modeOf(path.join(picked, APP, 'run.command'))).toBe(0o755);
    }
  });

  it('reports a missing app in the daemon’s own words', async () => {
    const fetch = vi.fn(async () => new Response("There is no app 'ghost'.", { status: 404 }));
    const { deps: d } = deps({ fetch: fetch as unknown as typeof globalThis.fetch });
    expect(await exportAppToFolder('ghost', launcher, d)).toEqual({
      status: 'failed',
      message: "There is no app 'ghost'.",
    });
    expect(await fs.readdir(picked)).toEqual([]);
  });

  it('refuses an app name that is a path, and a malformed request, before asking for a folder', async () => {
    const { deps: d } = deps();
    expect((await exportAppToFolder('../x', launcher, d)).status).toBe('failed');
    expect((await exportAppToFolder(APP, { mode: 'fat' }, d)).status).toBe('failed');
    expect((await exportAppToFolder(APP, { mode: 'full', bundleDaemon: false }, d)).status).toBe(
      'failed'
    );
    expect(d.chooseDirectory).not.toHaveBeenCalled();
  });
});

describe('extractExportZip', () => {
  it('refuses an entry that climbs out of the folder and writes nothing outside it', async () => {
    // adm-zip cleans a name it is given, so the hostile name is patched into the bytes.
    const zip = daemonZip({ 'aa/evil.txt': ['pwned', 0o644] });
    const hostile = Buffer.from(
      zip.toString('latin1').split(`${APP}/aa/evil.txt`).join(`${APP}/../evil.txt`),
      'latin1'
    );
    const staging = path.join(root, 'staging');
    await fs.mkdir(staging);
    expect(() => extractExportZip(hostile, APP, staging)).toThrow(/Unsafe zip entry/);
    await expect(fs.stat(path.join(root, 'evil.txt'))).rejects.toThrow();
  });

  it('refuses an entry outside the app folder, and skips a link', async () => {
    const staging = path.join(root, 'staging');
    await fs.mkdir(staging);
    expect(() =>
      extractExportZip(daemonZip({ 'x.txt': ['x', 0o644] }, 'someone-else'), APP, staging)
    ).toThrow(/outside its folder/);

    const zip = new AdmZip();
    zip.addFile(`${APP}/index.html`, Buffer.from('<html></html>'));
    const link = zip.addFile(`${APP}/hosts`, Buffer.from('/etc/hosts'));
    link.attr = ((0o120777 << 16) >>> 0) as number;
    const fresh = path.join(root, 'fresh');
    await fs.mkdir(fresh);
    const { files, skipped } = extractExportZip(zip.toBuffer(), APP, fresh);
    expect(files).toBe(1);
    expect(skipped).toEqual(['hosts: a link, never written']);
    await expect(fs.lstat(path.join(fresh, 'hosts'))).rejects.toThrow();
  });

  it('gives the launchers 0755 even when the zip recorded no mode', async () => {
    if (process.platform === 'win32') return;
    const zip = new AdmZip();
    zip.addFile(`${APP}/run.sh`, Buffer.from('#!/usr/bin/env bash\n'), '', 0);
    const entry = zip.getEntry(`${APP}/run.sh`);
    if (entry) entry.attr = 0;
    const staging = path.join(root, 'staging');
    await fs.mkdir(staging);
    extractExportZip(zip.toBuffer(), APP, staging);
    expect(await modeOf(path.join(staging, 'run.sh'))).toBe(0o755);
  });
});

describe('the apps:export handler in main.ts', () => {
  /**
   * The proof of a person must be the key the window's daemon was attached with, like its secret.
   * It was read from the settings at export time, so after a person saved Settings ▸ External
   * Backend under a running window the export sent another daemon's key, read as unproven, and
   * every private knowledge base the person asked for was left out.
   */
  it("sends the window's own daemon key, never one read from the settings now", async () => {
    const main = await fs.readFile(path.join(__dirname, '..', 'main.ts'), 'utf8');
    const start = main.indexOf("ipcMain.handle('apps:export'");
    expect(start).toBeGreaterThan(-1);
    const handler = main.slice(start, main.indexOf('\n});\n', start));
    expect(handler).toContain('userActionKey: daemon.userActionKey');
    expect(handler).not.toContain('loadSettings(');
    expect(handler).not.toContain('getUserActionKey(');
    expect(main).toMatch(/windowDaemons\.set\(mainWindow\.id, \{[^}]*\buserActionKey,/);
  });
});
