// @vitest-environment node
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import * as vm from 'node:vm';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';
import { crewFileRefusal, crewShareCopy } from './utils/crewSharePath';

const source = (name: string) =>
  fs.readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), name), 'utf8');

type PickerResult = { capability_id: string; name: string; size?: number } | null;
type PickerHandler = (
  event: { sender: { isDestroyed: () => boolean } },
  raw: unknown
) => Promise<PickerResult>;

type FetchCall = { url: string; method: string; body: string };
type MockResponse = { ok: boolean; json: () => Promise<unknown> };

const transferRequest = (overrides: Record<string, unknown> = {}) => ({
  direction: 'download',
  connectionId: 'conn-1',
  channelId: 'channel-1',
  expectedMode: 'private',
  suggestedName: 'report.txt',
  ...overrides,
});

function transferPickerJavaScript(): string {
  const main = source('main.ts');
  const start = main.indexOf("ipcMain.handle('crew:select-transfer-file'");
  const end = main.indexOf("ipcMain.handle('crew:authenticate'", start);
  if (start < 0 || end <= start) throw new Error('Could not locate transfer picker handler.');
  const registration = main.slice(start, end);
  const callbackStart = registration.indexOf('async (event');
  const callbackEnd = registration.lastIndexOf('});');
  if (callbackStart < 0 || callbackEnd <= callbackStart)
    throw new Error('Could not extract transfer picker callback.');
  const callback = registration.slice(callbackStart, callbackEnd + 1);
  const javascript = ts.transpileModule(`(${callback})`, {
    compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS },
  }).outputText;
  return javascript;
}

function createPickerHarness(
  responses: MockResponse[],
  options: {
    open?: { canceled: boolean; filePaths: string[] };
    save?: { canceled: boolean; filePath: string };
    messages?: number[];
    beforeFetch?: () => void;
  } = {}
) {
  let senderDestroyed = false;
  let responseIndex = 0;
  const calls: FetchCall[] = [];
  const dialogEvents: string[] = [];
  const sender = { isDestroyed: () => senderDestroyed };
  const owner = { id: 7, isDestroyed: () => senderDestroyed };
  const context = vm.createContext({
    AbortSignal,
    BrowserWindow: { fromWebContents: () => owner },
    biorouterdClients: new Map([[7, { getConfig: () => ({ baseUrl: 'http://daemon.test' }) }]]),
    dialog: {
      showOpenDialog: async () => {
        dialogEvents.push('open');
        return options.open ?? { canceled: false, filePaths: ['/tmp/report.txt'] };
      },
      showSaveDialog: async () => {
        dialogEvents.push('save');
        return options.save ?? { canceled: false, filePath: '/tmp/report.txt' };
      },
      showMessageBox: async (_window: unknown, message: { title: string }) => {
        dialogEvents.push(message.title);
        return { response: options.messages?.shift() ?? 0 };
      },
    },
    fetch: async (url: string, init: { method?: string; body?: string }) => {
      options.beforeFetch?.();
      calls.push({ url, method: init.method ?? 'GET', body: init.body ?? '' });
      const response = responses[responseIndex++];
      if (!response) throw new Error('The picker harness ran out of responses.');
      return response;
    },
    getServerSecret: () => 'server-secret',
    getUserActionKey: () => 'user-action',
    loadSettings: () => ({}),
    path,
    crewFileRefusal,
  });
  const runtimeHandler = vm.runInContext(transferPickerJavaScript(), context) as PickerHandler;
  return {
    calls,
    dialogEvents,
    destroySender: () => {
      senderDestroyed = true;
    },
    invoke: (raw: unknown) => runtimeHandler({ sender }, raw),
  };
}

const jsonResponse = (value: unknown, ok = true): MockResponse => ({
  ok,
  json: async () => value,
});

describe('Crew transfer picker opaque-capability bridge', () => {
  it('keeps selected filesystem paths in the main-process request and returns only the daemon capability', () => {
    const main = source('main.ts');
    const start = main.indexOf("ipcMain.handle('crew:select-transfer-file'");
    const end = main.indexOf("ipcMain.handle('crew:authenticate'", start);
    expect(start).toBeGreaterThanOrEqual(0);
    expect(end).toBeGreaterThan(start);
    const handler = main.slice(start, end);
    const returnStart = handler.lastIndexOf('return {');
    expect(handler).toMatch(/path: selected/);
    expect(handler).toMatch(/const purpose = options\.purpose \?\? 'transfer'/);
    expect(handler).toMatch(/purpose === 'cleanup'/);
    expect(handler).toMatch(/properties: \['openDirectory'\]/);
    expect(handler).toMatch(/typeof result\??\.capability_id !== 'string'/);
    expect(handler.slice(returnStart)).toMatch(
      /return \{\s*capability_id: result\.capability_id,\s*name: result\.name/
    );
    expect(handler.slice(returnStart)).not.toMatch(/selected|path|bytes|contents/);
    expect(handler).toMatch(/if \(!selected\) return null/);
  });

  it('passes only the opaque capability identifier from preload to Crew API calls', () => {
    const preload = source('preload.ts');
    const transfers = source('components/crew/crewTransfers.ts');
    const preloadStart = preload.indexOf('crewSelectTransferFile:');
    const preloadEnd = preload.indexOf('\n  ', preloadStart + 10);
    expect(preload.slice(preloadStart, preloadEnd)).not.toMatch(/path|bytes|contents/);
    expect(transfers).toMatch(/purpose: request\.purpose/);
    expect(transfers).toMatch(/file_capability: file\.capability_id/);
    expect(transfers).not.toMatch(/file\.(path|bytes|contents)/);
  });

  it('rejects an invalid privacy enum before opening a native picker and keeps daemon errors safe', () => {
    const main = source('main.ts');
    const start = main.indexOf("ipcMain.handle('crew:select-transfer-file'");
    const end = main.indexOf("ipcMain.handle('crew:authenticate'", start);
    const handler = main.slice(start, end);
    const validation = handler.indexOf('Invalid expected transfer privacy.');
    const firstDialog = handler.indexOf('dialog.showOpenDialog');
    expect(validation).toBeGreaterThanOrEqual(0);
    expect(firstDialog).toBeGreaterThan(validation);
    expect(handler).toMatch(/options\.expectedMode !== undefined/);
    expect(handler).toMatch(/options\.expectedMode !== 'private'/);
    expect(handler).toMatch(/options\.expectedMode !== 'public'/);
    expect(handler).toContain(
      'Connection privacy changed. Refresh Crew and choose the file again.'
    );
    expect(handler).toContain(
      'The daemon refused this file selection. Choose an accessible file or a new destination filename.'
    );
    expect(handler).not.toMatch(/throw new Error\(failure\?\.error/);
  });

  it('preflights a download before replacement approval and confirms the same capability', async () => {
    const harness = createPickerHarness(
      [
        jsonResponse({ capability_id: 'cap-1', name: 'report.txt', target_exists: true }),
        jsonResponse({ capability_id: 'cap-1', name: 'report.txt' }),
      ],
      { messages: [1] }
    );

    await expect(harness.invoke(transferRequest())).resolves.toEqual({
      capability_id: 'cap-1',
      name: 'report.txt',
    });
    expect(harness.calls).toHaveLength(2);
    expect(harness.calls[0].url).toBe('http://daemon.test/crew/files');
    expect(JSON.parse(harness.calls[0].body)).toMatchObject({
      path: '/tmp/report.txt',
      approval_pending: true,
      overwrite: true,
    });
    expect(harness.calls[1]).toMatchObject({
      url: 'http://daemon.test/crew/files/cap-1/confirm',
      method: 'POST',
      body: '{}',
    });
    expect(harness.dialogEvents).toEqual(['save', 'Replace Crew download destination']);
  });

  it('deletes a declined replacement preflight without confirming or re-registering it', async () => {
    const harness = createPickerHarness(
      [
        jsonResponse({ capability_id: 'cap-2', name: 'report.txt', target_exists: true }),
        jsonResponse(null),
      ],
      { messages: [0] }
    );

    await expect(harness.invoke(transferRequest())).resolves.toBeNull();
    expect(harness.calls).toHaveLength(2);
    expect(harness.calls[1]).toMatchObject({
      url: 'http://daemon.test/crew/files/cap-2',
      method: 'DELETE',
      body: '{}',
    });
    expect(harness.calls.some((call) => call.url.endsWith('/confirm'))).toBe(false);
  });

  it('reports stale destination refusal safely and leaves upload and cleanup flows independent', async () => {
    const stale = createPickerHarness([
      jsonResponse({ error: 'destination changed on the server' }, false),
    ]);
    await expect(stale.invoke(transferRequest())).rejects.toThrow(
      'The daemon refused this file selection. Choose an accessible file or a new destination filename.'
    );
    expect(stale.dialogEvents).toEqual(['save']);

    const upload = createPickerHarness(
      [jsonResponse({ capability_id: 'upload-1', name: 'upload.txt' })],
      { open: { canceled: false, filePaths: ['/tmp/upload.txt'] } }
    );
    await expect(upload.invoke(transferRequest({ direction: 'upload' }))).resolves.toEqual({
      capability_id: 'upload-1',
      name: 'upload.txt',
    });
    expect(upload.dialogEvents).toEqual(['open']);
    expect(JSON.parse(upload.calls[0].body)).toMatchObject({
      purpose: 'transfer',
      approval_pending: false,
      overwrite: false,
    });

    const cleanup = createPickerHarness(
      [jsonResponse({ capability_id: 'cleanup-1', name: 'report.txt' })],
      { open: { canceled: false, filePaths: ['/tmp'] }, messages: [1] }
    );
    await expect(
      cleanup.invoke(transferRequest({ purpose: 'cleanup', transferId: 'transfer-1' }))
    ).resolves.toEqual({ capability_id: 'cleanup-1', name: 'report.txt' });
    expect(cleanup.dialogEvents).toEqual(['open', 'Remove incomplete Crew download']);
    expect(JSON.parse(cleanup.calls[0].body)).toMatchObject({
      purpose: 'cleanup',
      approval_pending: false,
      overwrite: false,
    });
  });

  it('names a credential refusal in plain words for an upload and a download (Q3-01)', async () => {
    const refusal = jsonResponse(
      { code: 'crew_file_is_credential', error: 'daemon wording is never relayed' },
      false
    );
    const upload = createPickerHarness([refusal], {
      open: { canceled: false, filePaths: ['/Users/frank/profile/biorouter/config/secrets.yaml'] },
    });
    await expect(upload.invoke(transferRequest({ direction: 'upload' }))).rejects.toThrow(
      crewShareCopy.credential('secrets.yaml')
    );
    expect(upload.calls).toHaveLength(1);

    const download = createPickerHarness([refusal], {
      save: { canceled: false, filePath: '/Users/frank/.ssh/id_ed25519' },
    });
    await expect(download.invoke(transferRequest())).rejects.toThrow(
      crewShareCopy.credentialLocation
    );
    expect(download.dialogEvents).toEqual(['save']);
    expect(download.calls).toHaveLength(1);
  });

  it('stops before replacement UI when the sender closes after preflight', async () => {
    const harness = createPickerHarness(
      [jsonResponse({ capability_id: 'cap-3', name: 'report.txt', target_exists: true })],
      { beforeFetch: () => harness.destroySender() }
    );

    await expect(harness.invoke(transferRequest())).rejects.toThrow(
      'The file selection window closed.'
    );
    expect(harness.dialogEvents).toEqual(['save']);
  });
});
