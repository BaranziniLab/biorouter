// @vitest-environment node
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import * as vm from 'node:vm';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as crewSharePath from './crewSharePath';
import { CREW_SHARE_DROPPED_FILE_CHANNEL, createCrewShareDroppedFile } from './crewSharePathBridge';

type Invoke = (channel: string, ...args: unknown[]) => Promise<unknown>;

const source = (relative: string) =>
  fs.readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), relative), 'utf8');

const destination = {
  connectionId: 'conn-1',
  channelId: 'chan-1',
  channelName: 'methods',
  workspaceName: 'chen-lab',
  expectedMode: 'private' as const,
};

describe('the preload bridge', () => {
  it('resolves the path from the File itself and sends only the named fields', async () => {
    const file = { name: 'growth.csv' } as File;
    const getPathForFile = vi.fn(() => '/Users/frank/Desktop/growth.csv');
    const invoke = vi.fn<Invoke>(async () => ({ outcome: 'cancelled' }));
    const share = createCrewShareDroppedFile({ getPathForFile }, { invoke });

    await expect(
      share(file, {
        ...destination,
        path: '/etc/passwd',
        capability_id: 'forged',
      } as unknown as typeof destination)
    ).resolves.toEqual({ outcome: 'cancelled' });

    expect(getPathForFile).toHaveBeenCalledWith(file);
    expect(invoke).toHaveBeenCalledWith(CREW_SHARE_DROPPED_FILE_CHANNEL, {
      path: '/Users/frank/Desktop/growth.csv',
      connectionId: 'conn-1',
      channelId: 'chan-1',
      channelName: 'methods',
      workspaceName: 'chen-lab',
      expectedMode: 'private',
    });
  });

  it('sends an empty path when the File has no file behind it', async () => {
    const invoke = vi.fn<Invoke>(async () => ({ outcome: 'refused', message: 'x' }));
    const throwing = createCrewShareDroppedFile(
      {
        getPathForFile: () => {
          throw new TypeError('not a File');
        },
      },
      { invoke }
    );
    await throwing({} as File, destination);
    const odd = createCrewShareDroppedFile(
      { getPathForFile: () => 42 as unknown as string },
      { invoke }
    );
    await odd({} as File, destination);
    expect(invoke.mock.calls.map((call) => (call[1] as { path: string }).path)).toEqual(['', '']);
  });

  it('is what preload.ts exposes, on the channel main.ts answers', () => {
    const preload = source('../preload.ts');
    expect(preload).toContain(
      "import { createCrewShareDroppedFile } from './utils/crewSharePathBridge';"
    );
    expect(preload).toContain(
      'crewShareDroppedFile: createCrewShareDroppedFile(webUtils, ipcRenderer),'
    );
    expect(preload).toContain(
      "crewShareDroppedFile: import('./utils/crewSharePathBridge').CrewShareDroppedFile;"
    );
    const main = source('../main.ts');
    expect(main).toContain('ipcMain.handle(CREW_SHARE_DROPPED_FILE_CHANNEL,');
    expect(CREW_SHARE_DROPPED_FILE_CHANNEL).toBe('crew:share-dropped-file');
  });

  it('keeps the bridge free of Node imports, so a sandboxed preload can bundle it', () => {
    const bridge = source('crewSharePathBridge.ts');
    expect(bridge).not.toMatch(/^import (?!type )/m);
  });
});

/**
 * The `main.ts` handler, run for real in a VM with Electron and the daemon faked.
 *
 * Extracted by source the way `crewTransferPickerBridge.test.ts` extracts the Attach picker, so
 * the test exercises the code that ships rather than a copy of it.
 */
function mainShareHandlerJavaScript(): string {
  const main = source('../main.ts');
  const start = main.indexOf('let crewShareAutoConfirm = false;');
  const end = main.indexOf('\nfunction registerCliInstallHandlers()', start);
  if (start < 0 || end <= start) throw new Error('Could not locate the Crew share handler.');
  const script = `${main.slice(start, end)}
;({ register: registerCrewShareHandler, setAutoConfirm: (value) => { crewShareAutoConfirm = value; } })`;
  return ts.transpileModule(script, {
    compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS },
  }).outputText;
}

type Handler = (
  event: { sender: { id: number; isDestroyed: () => boolean } },
  raw: unknown
) => Promise<unknown>;

function createMainHarness(
  options: {
    responses?: Array<{ ok: boolean; body: unknown }>;
    dialog?: (owner: unknown, options: { message: string; detail: string }) => Promise<number>;
    baseUrl?: string | null;
  } = {}
) {
  const handlers = new Map<string, Handler>();
  const fetchCalls: Array<{
    url: string;
    method: string;
    headers: Record<string, string>;
    body: unknown;
  }> = [];
  const dialogs: Array<{ owner: unknown; message: string; detail: string }> = [];
  const logs: string[] = [];
  const owner = { id: 7, isDestroyed: () => false };
  const responses = [...(options.responses ?? [])];
  const context = vm.createContext({
    AbortSignal,
    encodeURIComponent,
    JSON,
    ipcMain: { handle: (channel: string, handler: Handler) => handlers.set(channel, handler) },
    BrowserWindow: { fromWebContents: () => owner },
    biorouterdClients: new Map(
      options.baseUrl === null
        ? []
        : [[7, { getConfig: () => ({ baseUrl: options.baseUrl ?? 'http://daemon.test' }) }]]
    ),
    dialog: {
      showMessageBox: async (parent: unknown, box: { message: string; detail: string }) => {
        dialogs.push({ owner: parent, message: box.message, detail: box.detail });
        return { response: options.dialog ? await options.dialog(parent, box) : 0 };
      },
    },
    fetch: async (
      url: string,
      init: { method: string; headers: Record<string, string>; body: string }
    ) => {
      fetchCalls.push({
        url,
        method: init.method,
        headers: init.headers,
        body: JSON.parse(init.body),
      });
      const next = responses.shift();
      if (!next) throw new Error('The share harness ran out of daemon responses.');
      return { ok: next.ok, json: async () => next.body };
    },
    loadSettings: () => ({}),
    getServerSecret: () => 'server-secret',
    getUserActionKey: () => 'user-action',
    log: { warn: (message: string) => logs.push(message) },
    CREW_SHARE_DROPPED_FILE_CHANNEL,
    CrewSharePending: crewSharePath.CrewSharePending,
    crewShareCopy: crewSharePath.crewShareCopy,
    parseCrewShareRequest: crewSharePath.parseCrewShareRequest,
    shareDroppedFile: crewSharePath.shareDroppedFile,
  });
  const exported = vm.runInContext(mainShareHandlerJavaScript(), context) as {
    register: () => void;
    setAutoConfirm: (value: boolean) => void;
  };
  exported.register();
  const handler = handlers.get(CREW_SHARE_DROPPED_FILE_CHANNEL);
  if (!handler) throw new Error('The share handler did not register.');
  const sender = { id: 11, isDestroyed: () => false };
  return {
    owner,
    fetchCalls,
    dialogs,
    logs,
    setAutoConfirm: exported.setAutoConfirm,
    invoke: (raw: unknown) => handler({ sender }, raw),
  };
}

describe('the main.ts share handler', () => {
  let root: string;
  let file: string;
  beforeEach(async () => {
    root = await fsp.realpath(await fsp.mkdtemp(path.join(os.tmpdir(), 'crew-share-main-')));
    file = path.join(root, 'growth.csv');
    await fsp.writeFile(file, 'a,b\n1,2\n');
  });
  afterEach(async () => {
    await fsp.rm(root, { recursive: true, force: true });
  });
  const request = () => ({ ...destination, path: file });
  const capability = { ok: true, body: { capability_id: 'cap-1', name: 'growth.csv', size: 8 } };

  it('asks in a dialog parented to the window, then registers with the daemon secret and proof', async () => {
    const harness = createMainHarness({ responses: [capability] });
    await expect(harness.invoke(request())).resolves.toEqual({
      outcome: 'shared',
      capability_id: 'cap-1',
      name: 'growth.csv',
      size: 8,
    });
    expect(harness.dialogs).toEqual([
      {
        owner: harness.owner,
        message: 'Share "growth.csv" (8 bytes) to #methods in chen-lab?',
        detail: `Full path: ${file}`,
      },
    ]);
    expect(harness.fetchCalls).toEqual([
      {
        url: 'http://daemon.test/crew/files',
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'X-Secret-Key': 'server-secret',
          'X-User-Action': 'user-action',
        },
        body: {
          direction: 'upload',
          purpose: 'transfer',
          path: file,
          overwrite: false,
          approval_pending: false,
          connection_id: 'conn-1',
          channel_id: 'chan-1',
          expected_mode: 'private',
        },
      },
    ]);
    expect(harness.logs).toEqual([]);
  });

  it('reaches no daemon after Cancel', async () => {
    const harness = createMainHarness({ dialog: async () => 1 });
    await expect(harness.invoke(request())).resolves.toEqual({ outcome: 'cancelled' });
    expect(harness.fetchCalls).toEqual([]);
  });

  it('throws on a malformed request before any dialog', async () => {
    const harness = createMainHarness();
    await expect(harness.invoke({ ...request(), channelId: '../x' })).rejects.toThrow(
      'Invalid transfer destination.'
    );
    await expect(harness.invoke({ ...request(), expectedMode: 'secret' })).rejects.toThrow();
    expect(harness.dialogs).toEqual([]);
  });

  it('throws without a daemon for the window', async () => {
    const harness = createMainHarness({ baseUrl: null });
    await expect(harness.invoke(request())).rejects.toThrow('The local daemon is not available.');
    expect(harness.dialogs).toEqual([]);
  });

  it('refuses a second share while one confirmation is open, and allows one after', async () => {
    let answer: (value: number) => void = () => undefined;
    let held = false;
    const harness = createMainHarness({
      responses: [capability],
      // The first dialog stays open until `answer`; any later one is cancelled at once.
      dialog: () => {
        if (held) return Promise.resolve(1);
        held = true;
        return new Promise<number>((resolve) => (answer = resolve));
      },
    });
    const first = harness.invoke(request());
    await vi.waitFor(() => expect(harness.dialogs).toHaveLength(1));
    await expect(harness.invoke(request())).resolves.toEqual({
      outcome: 'refused',
      message: crewSharePath.crewShareCopy.busy,
    });
    expect(harness.dialogs).toHaveLength(1);
    answer(1);
    await expect(first).resolves.toEqual({ outcome: 'cancelled' });
    await expect(harness.invoke(request())).resolves.toEqual({ outcome: 'cancelled' });
  });

  it('gives a mismatched capability back with DELETE', async () => {
    const harness = createMainHarness({
      responses: [
        { ok: true, body: { capability_id: 'cap-2', name: 'growth.csv', size: 99 } },
        { ok: true, body: { discarded: true } },
      ],
    });
    await expect(harness.invoke(request())).resolves.toMatchObject({ outcome: 'refused' });
    expect(harness.fetchCalls[1]).toMatchObject({
      url: 'http://daemon.test/crew/files/cap-2',
      method: 'DELETE',
    });
  });

  it('under the development auto-confirm, shows no dialog and logs the confirmed path', async () => {
    const harness = createMainHarness({ responses: [capability] });
    harness.setAutoConfirm(true);
    await expect(harness.invoke(request())).resolves.toMatchObject({ outcome: 'shared' });
    expect(harness.dialogs).toEqual([]);
    expect(harness.logs).toHaveLength(1);
    expect(harness.logs[0]).toContain(file);
  });

  it('keeps the path out of what the renderer gets back', async () => {
    const harness = createMainHarness({ responses: [capability] });
    const result = await harness.invoke(request());
    expect(JSON.stringify(result)).not.toContain(root);
  });
});

describe('the development auto-confirm gate in main.ts', () => {
  const main = source('../main.ts');
  const call = (name: string) => {
    const start = main.indexOf(`${name}({`);
    if (start < 0) throw new Error(`${name} is not called in main.ts`);
    return main.slice(start, main.indexOf('\n  });', start));
  };

  it('is fed the same four conditions as the development approval stdin', () => {
    const approval = call('createDevelopmentApprovalReader');
    const share = call('resolveDevAutoConfirmShare');
    for (const condition of [
      'isPackaged: app.isPackaged,',
      'developmentProfileRoot,',
      'testDriverEnabled: Boolean(process.env.ENABLE_PLAYWRIGHT),',
      'sharedDaemonEnabled: isSharedDaemonEnabled() && !loadSettings().externalBiorouterd?.enabled,',
    ]) {
      expect(approval).toContain(condition);
      expect(share).toContain(condition);
    }
    expect(share).toContain('value: process.env[DEV_AUTO_CONFIRM_SHARE_ENV],');
    expect(crewSharePath.DEV_AUTO_CONFIRM_SHARE_ENV).toBe('BIOROUTER_DEV_AUTO_CONFIRM_SHARE');
  });

  it('is the only thing that can turn the auto-confirm on', () => {
    const assignments = main.match(/(?<!let )crewShareAutoConfirm = /g) ?? [];
    expect(assignments).toHaveLength(1);
    expect(main).toContain('let crewShareAutoConfirm = false;');
    expect(main).toContain('crewShareAutoConfirm = crewShareAutoConfirmGate.enabled;');
  });
});
