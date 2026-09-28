import { promises as fs } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  LAUNCH_PAGE_STALE_MS,
  openAppInSystemBrowser,
  writeAppLaunchPage,
} from './appBrowserLaunch';

const base = 'http://127.0.0.1:64005';
const token = 'ef'.repeat(32);
const link = `${base}/apps/cohort-explorer/?t=${token}`;

function daemonAnswering(body: unknown, status = 200) {
  return vi.fn(
    async () =>
      ({
        ok: status >= 200 && status < 300,
        status,
        json: async () => body,
      }) as Response
  );
}

let scratch: string;
beforeEach(async () => {
  scratch = await fs.mkdtemp(path.join(os.tmpdir(), 'app-launch-test-'));
});
afterEach(async () => {
  await fs.rm(scratch, { recursive: true, force: true });
});

const unix = process.platform !== 'win32';
const mode = async (target: string) => (await fs.stat(target)).mode & 0o777;

describe('writeAppLaunchPage', () => {
  it('writes a page only this account can read, which forwards to the link', async () => {
    const directory = path.join(scratch, 'app-launch');
    const page = await writeAppLaunchPage(directory, link);
    expect(path.dirname(page)).toBe(directory);
    expect(path.basename(page)).toMatch(/^launch-[0-9a-f]{32}\.html$/);
    expect(page).not.toContain('?t=');
    const html = await fs.readFile(page, 'utf8');
    expect(html).toContain(`<meta http-equiv="refresh" content="0;url=${link}">`);
    expect(html).toContain(`<a href="${link}">`);
    expect(html).toContain('<meta name=referrer content=no-referrer>');
    if (unix) {
      expect(await mode(page)).toBe(0o600);
      expect(await mode(directory)).toBe(0o700);
    }
  });

  it.skipIf(!unix)('tightens a folder of ours that others could enter', async () => {
    const directory = path.join(scratch, 'app-launch');
    await fs.mkdir(directory, { mode: 0o755 });
    await fs.chmod(directory, 0o755);
    await writeAppLaunchPage(directory, link);
    expect(await mode(directory)).toBe(0o700);
  });

  it.skipIf(!unix)('refuses a folder that is a link', async () => {
    const elsewhere = path.join(scratch, 'elsewhere');
    await fs.mkdir(elsewhere);
    const directory = path.join(scratch, 'app-launch');
    await fs.symlink(elsewhere, directory);
    await expect(writeAppLaunchPage(directory, link)).rejects.toThrow('not a plain folder');
    expect(await fs.readdir(elsewhere)).toEqual([]);
  });

  it('removes pages whose links expired long ago, and nothing else', async () => {
    const directory = path.join(scratch, 'app-launch');
    const old = await writeAppLaunchPage(directory, link);
    const recent = await writeAppLaunchPage(directory, link);
    const other = path.join(directory, 'notes.txt');
    await fs.writeFile(other, 'keep');
    const longAgo = new Date(Date.now() - LAUNCH_PAGE_STALE_MS - 1000);
    await fs.utimes(old, longAgo, longAgo);
    await fs.utimes(other, longAgo, longAgo);
    const next = await writeAppLaunchPage(directory, link);
    const left = await fs.readdir(directory);
    expect(left).not.toContain(path.basename(old));
    expect(left).toEqual(
      expect.arrayContaining([path.basename(recent), path.basename(next), 'notes.txt'])
    );
  });
});

describe('openAppInSystemBrowser', () => {
  it('hands the opener the private page and never the link', async () => {
    const opened: string[] = [];
    await openAppInSystemBrowser({
      baseUrl: base,
      appId: 'cohort-explorer',
      secretKey: 'daemon-secret',
      directory: path.join(scratch, 'app-launch'),
      openPath: async (page) => {
        opened.push(page);
        return '';
      },
      fetchImpl: daemonAnswering({ path: `/apps/cohort-explorer/?t=${token}` }),
    });
    expect(opened).toHaveLength(1);
    expect(opened[0]).not.toContain('?t=');
    expect(opened[0]).not.toContain(token);
    expect(await fs.readFile(opened[0], 'utf8')).toContain(link);
  });

  it('opens nothing when the daemon will not mint a link', async () => {
    const openPath = vi.fn(async () => '');
    await expect(
      openAppInSystemBrowser({
        baseUrl: base,
        appId: 'cohort-explorer',
        secretKey: 'daemon-secret',
        directory: path.join(scratch, 'app-launch'),
        openPath,
        fetchImpl: daemonAnswering('no', 403),
      })
    ).rejects.toThrow('would not open this app');
    expect(openPath).not.toHaveBeenCalled();
  });

  it('reports an opener that failed', async () => {
    await expect(
      openAppInSystemBrowser({
        baseUrl: base,
        appId: 'cohort-explorer',
        secretKey: 'daemon-secret',
        directory: path.join(scratch, 'app-launch'),
        openPath: async () => 'no application knows how to open this file',
        fetchImpl: daemonAnswering({ path: `/apps/cohort-explorer/?t=${token}` }),
      })
    ).rejects.toThrow('could not be opened');
  });
});
