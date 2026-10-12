import { promises as fs } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  CONFINED_BROWSER_MESSAGE,
  LAUNCH_PAGE_STALE_MS,
  findDefaultBrowser,
  launchFile,
  openAppInSystemBrowser,
  openPageInBrowser,
  openPageInDefaultBrowser,
  writeAppLaunchPage,
  type DefaultBrowserOpener,
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
      daemonListener: true,
      directory: path.join(scratch, 'app-launch'),
      openPage: async (page) => {
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

  /**
   * The reported bug: on a shared daemon the window talks to a proxy that refuses a browser, and
   * the page sent the browser there ("Access to 127.0.0.1 was denied").
   */
  it('mints through the proxy and sends the browser to the daemon’s own listener', async () => {
    const proxy = 'http://127.0.0.1:51852';
    const daemonLink = `http://127.0.0.1:51851/apps/cohort-explorer/?t=${token}`;
    const daemon = daemonAnswering({ path: `/apps/cohort-explorer/?t=${token}`, url: daemonLink });
    const opened: string[] = [];
    await openAppInSystemBrowser({
      baseUrl: proxy,
      appId: 'cohort-explorer',
      secretKey: 'daemon-secret',
      daemonListener: true,
      directory: path.join(scratch, 'app-launch'),
      openPage: async (page) => {
        opened.push(page);
        return '';
      },
      fetchImpl: daemon,
    });
    expect(daemon).toHaveBeenCalledWith(`${proxy}/apps/cohort-explorer/launch`, expect.anything());
    const html = await fs.readFile(opened[0], 'utf8');
    expect(html).toContain(`content="0;url=${daemonLink}"`);
    expect(html).not.toContain(proxy);
  });

  it('opens nothing when the daemon will not mint a link', async () => {
    const openPage = vi.fn(async () => '');
    await expect(
      openAppInSystemBrowser({
        baseUrl: base,
        appId: 'cohort-explorer',
        secretKey: 'daemon-secret',
        daemonListener: true,
        directory: path.join(scratch, 'app-launch'),
        openPage,
        fetchImpl: daemonAnswering('no', 403),
      })
    ).rejects.toThrow("refused this window's connection");
    expect(openPage).not.toHaveBeenCalled();
  });

  /**
   * A snap browser cannot read the page, so the person is asked to take the link over instead.
   * The link is minted only on their yes, so its five minutes start when they agree.
   */
  it('hands the person the link for a browser that cannot read the page, after asking', async () => {
    const directory = path.join(scratch, 'app-launch');
    const openPage = vi.fn(async () => '');
    const copy = vi.fn();
    const daemon = daemonAnswering({ path: `/apps/cohort-explorer/?t=${token}` });
    const options = {
      baseUrl: base,
      appId: 'cohort-explorer',
      secretKey: 'daemon-secret',
      daemonListener: true,
      directory,
      openPage,
      fetchImpl: daemon,
    };

    await openAppInSystemBrowser({ ...options, copyLink: { confirm: async () => false, copy } });
    expect(daemon).not.toHaveBeenCalled();
    expect(copy).not.toHaveBeenCalled();

    await openAppInSystemBrowser({ ...options, copyLink: { confirm: async () => true, copy } });
    expect(copy).toHaveBeenCalledWith(link);
    expect(openPage).not.toHaveBeenCalled();
    await expect(fs.readdir(directory)).rejects.toThrow();
  });

  it('reports an opener that failed', async () => {
    await expect(
      openAppInSystemBrowser({
        baseUrl: base,
        appId: 'cohort-explorer',
        secretKey: 'daemon-secret',
        daemonListener: true,
        directory: path.join(scratch, 'app-launch'),
        openPage: async () => 'no application knows how to open this file',
        fetchImpl: daemonAnswering({ path: `/apps/cohort-explorer/?t=${token}` }),
      })
    ).rejects.toThrow('could not be opened');
  });
});

/**
 * `shell.openPath` opens a page with whatever opens `.html` files, which on a developer's
 * machine is often an editor: it showed the page's source and reported success.
 */
describe('openPageInDefaultBrowser', () => {
  const page = '/Users/someone/Library/Application Support/Biorouter/app-launch/launch-x.html';

  function opener(
    platform: typeof process.platform,
    overrides: Partial<DefaultBrowserOpener> = {}
  ) {
    const calls: Array<{ file: string; args: string[] }> = [];
    const deps: DefaultBrowserOpener = {
      platform,
      applicationForProtocol: vi.fn(async () => ({
        path:
          platform === 'win32'
            ? 'C:\\Program Files\\Mozilla Firefox\\firefox.exe'
            : '/Applications/Firefox.app',
      })),
      run: vi.fn(async (file: string, args: string[]) => {
        calls.push({ file, args });
        return file === 'xdg-settings' ? 'firefox.desktop\n' : '';
      }),
      launch: vi.fn(async (file: string, args: string[]) => {
        calls.push({ file, args });
      }),
      openPath: vi.fn(async () => ''),
      ...overrides,
    };
    return { deps, calls };
  }

  it('opens the page in the https: handler on macOS, never the .html one', async () => {
    const { deps, calls } = opener('darwin');
    await expect(openPageInDefaultBrowser(page, deps)).resolves.toBe('');
    expect(deps.applicationForProtocol).toHaveBeenCalledWith('https://');
    expect(calls).toEqual([
      { file: '/usr/bin/open', args: ['-a', '/Applications/Firefox.app', page] },
    ]);
    expect(deps.openPath).not.toHaveBeenCalled();
  });

  it('opens the page in the desktop’s default web browser on Linux', async () => {
    const { deps, calls } = opener('linux');
    await expect(openPageInDefaultBrowser(page, deps)).resolves.toBe('');
    expect(calls).toEqual([
      { file: 'xdg-settings', args: ['get', 'default-web-browser'] },
      { file: 'gtk-launch', args: ['firefox.desktop', page] },
    ]);
    expect(deps.openPath).not.toHaveBeenCalled();
  });

  /**
   * Windows used `shell.openPath`, which opens a page with whatever opens `.html` files: an
   * editor showed the page's source and reported success. The browser's own program is started
   * and left running; waiting on it would end it at the opener's timeout.
   */
  it('opens the page in the https: handler’s program on Windows', async () => {
    const { deps, calls } = opener('win32');
    await expect(openPageInDefaultBrowser(page, deps)).resolves.toBe('');
    expect(deps.applicationForProtocol).toHaveBeenCalledWith('https://');
    expect(deps.launch).toHaveBeenCalledWith('C:\\Program Files\\Mozilla Firefox\\firefox.exe', [
      page,
    ]);
    expect(deps.run).not.toHaveBeenCalled();
    expect(calls).toHaveLength(1);
    expect(deps.openPath).not.toHaveBeenCalled();
  });

  /**
   * Stock Ubuntu's Firefox is a snap, whose sandbox cannot read hidden folders. `gtk-launch`
   * handed it the page and returned success while the browser showed "Access denied".
   */
  it.each(['firefox_firefox.desktop', 'chromium_chromium.desktop', 'brave_brave.desktop'])(
    'neither launches a snap browser (%s) nor reports success',
    async (id) => {
      const { deps, calls } = opener('linux', { run: vi.fn(async () => `${id}\n`) });
      await expect(findDefaultBrowser(deps)).resolves.toEqual({ kind: 'confined', id });
      await expect(openPageInDefaultBrowser(page, deps)).resolves.toBe(CONFINED_BROWSER_MESSAGE);
      expect(vi.mocked(deps.run).mock.calls.map(([file]) => file)).not.toContain('gtk-launch');
      expect(calls).toEqual([]);
      expect(deps.openPath).not.toHaveBeenCalled();
    }
  );

  it.each([
    'firefox.desktop',
    'google-chrome.desktop',
    'org.mozilla.firefox.desktop', // Flatpak: the page reaches it through the document portal
    'brave-browser.desktop',
  ])('launches %s, which can read the page', async (id) => {
    const { deps } = opener('linux', { run: vi.fn(async () => `${id}\n`) });
    await expect(findDefaultBrowser(deps)).resolves.toEqual({ kind: 'desktop', id });
    await expect(openPageInBrowser(page, { kind: 'desktop', id }, deps)).resolves.toBe('');
    expect(deps.run).toHaveBeenCalledWith('gtk-launch', [id, page]);
  });

  it.each([
    [
      'darwin',
      {
        applicationForProtocol: vi.fn(async () => {
          throw new Error('no handler');
        }),
      },
    ],
    ['darwin', { applicationForProtocol: vi.fn(async () => ({ path: '' })) }],
    [
      'linux',
      {
        run: vi.fn(async (file: string) => {
          if (file === 'gtk-launch') throw new Error('gtk-launch: not found');
          return 'firefox.desktop';
        }),
      },
    ],
    // Something that is not a desktop id is never handed to gtk-launch as one.
    ['linux', { run: vi.fn(async () => '--version') }],
    // Nor something that is not a program run as one.
    ['win32', { applicationForProtocol: vi.fn(async () => ({ path: 'MSEdgeHTM' })) }],
    ['win32', { applicationForProtocol: vi.fn(async () => ({ path: '' })) }],
    [
      'win32',
      {
        launch: vi.fn(async () => {
          throw new Error('spawn ENOENT');
        }),
      },
    ],
    ['freebsd', {}],
  ] as Array<[typeof process.platform, Partial<DefaultBrowserOpener>]>)(
    'falls back to the generic opener on %s when the browser cannot be found that way',
    async (platform, overrides) => {
      const { deps } = opener(platform, overrides);
      await expect(openPageInDefaultBrowser(page, deps)).resolves.toBe('');
      expect(deps.openPath).toHaveBeenCalledWith(page);
    }
  );

  it('hands every program the page’s path and nothing that carries the link', async () => {
    for (const platform of ['darwin', 'linux', 'win32'] as (typeof process.platform)[]) {
      const { deps, calls } = opener(platform);
      await openPageInDefaultBrowser(page, deps);
      const handed = [
        ...calls.flatMap((call) => [call.file, ...call.args]),
        ...vi.mocked(deps.openPath).mock.calls.flat(),
      ];
      expect(handed.join(' ')).not.toMatch(/\?t=|https?:/);
    }
  });
});

/**
 * On Windows the browser's own program is started with the page. Waiting for it, as `runFile`
 * does, would end a browser that was not already running at the opener's timeout.
 */
describe('launchFile', () => {
  it('starts a program and leaves it running', async () => {
    const pidFile = path.join(scratch, 'pid');
    const script = `require('fs').writeFileSync(${JSON.stringify(pidFile)}, String(process.pid)); setTimeout(() => {}, 60000);`;
    await launchFile(process.execPath, ['-e', script]);
    await vi.waitFor(async () => expect(await fs.readFile(pidFile, 'utf8')).toMatch(/^\d+$/), {
      timeout: 10_000,
    });
    const pid = Number(await fs.readFile(pidFile, 'utf8'));
    expect(() => process.kill(pid, 0)).not.toThrow();
    process.kill(pid);
  });

  it('reports a program that could not be started', async () => {
    await expect(launchFile(path.join(scratch, 'no-such-browser'), [])).rejects.toThrow();
  });
});
