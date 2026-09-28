// @vitest-environment node
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import type { MessageBoxOptions } from 'electron';
import { describe, expect, it } from 'vitest';
import {
  CREW_FILE_IS_CREDENTIAL,
  CREW_FILE_NAME_HIDDEN,
  CREW_FOLDER_SHARED,
  CREW_MODE_MISMATCH,
  DAEMON_SENTENCE_MAX_CHARS,
  crewFileRefusal,
  crewShareCopy,
  parseCrewPickerRequest,
  selectCrewTransferFile,
  type CrewPickerDeps,
} from './utils/crewSharePath';

/**
 * The security contract of `crew:select-transfer-file`, the Attach picker, the Save window and the
 * cleanup window: the chosen path stays in the main process and only the daemon's opaque
 * capability goes back to the renderer.
 *
 * The flow lives in `utils/crewSharePath.ts` (`parseCrewPickerRequest`, `selectCrewTransferFile`),
 * where it runs here against stubbed native windows and a stubbed daemon; `main.ts` keeps only the
 * wiring, which the source checks below pin: the request is validated before any window opens,
 * the whole flow runs inside `holdCrewSheet`, and every daemon call carries the secret and the
 * user-action proof.
 */

const source = (name: string) =>
  fs.readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), name), 'utf8');

/** The `crew:select-transfer-file` registration in `main.ts`. */
function pickerHandlerSource(): string {
  const main = source('main.ts');
  const start = main.indexOf("ipcMain.handle('crew:select-transfer-file'");
  const end = main.indexOf("ipcMain.handle('crew:authenticate'", start);
  if (start < 0 || end <= start) throw new Error('Could not locate transfer picker handler.');
  return main.slice(start, end);
}

/** One exported function of `utils/crewSharePath.ts`, up to the next top-level declaration. */
function sharePathFunction(name: string): string {
  const module = source('utils/crewSharePath.ts');
  const start = module.search(new RegExp(`export (async )?function ${name}\\b`));
  if (start < 0) throw new Error(`Could not locate ${name}.`);
  const next = module.slice(start + 1).search(/\n(export |const |function |\/\*\*)/);
  return next < 0 ? module.slice(start) : module.slice(start, start + 1 + next);
}

type DaemonAnswer = { ok: boolean; body: unknown };
type DaemonCall = { endpoint: string; method: 'POST' | 'DELETE'; body: unknown };

const transferRequest = (overrides: Record<string, unknown> = {}) => ({
  direction: 'download',
  connectionId: 'conn-1',
  channelId: 'channel-1',
  expectedMode: 'private',
  suggestedName: 'report.txt',
  ...overrides,
});

/**
 * The picker against stubbed native windows (each one recorded by what it showed) and a daemon
 * that answers `responses` in order.
 */
function createPickerHarness(
  responses: DaemonAnswer[],
  options: {
    open?: { canceled: boolean; filePaths: string[] };
    save?: { canceled: boolean; filePath: string };
    messages?: number[];
    beforeFetch?: () => void;
  } = {}
) {
  let closed = false;
  let responseIndex = 0;
  const calls: DaemonCall[] = [];
  const dialogEvents: string[] = [];
  const deps: CrewPickerDeps = {
    showOpenDialog: async () => {
      dialogEvents.push('open');
      return options.open ?? { canceled: false, filePaths: ['/tmp/report.txt'] };
    },
    showSaveDialog: async () => {
      dialogEvents.push('save');
      return options.save ?? { canceled: false, filePath: '/tmp/report.txt' };
    },
    showMessageBox: async (message: MessageBoxOptions) => {
      dialogEvents.push(message.title ?? '');
      return { response: options.messages?.shift() ?? 0 };
    },
    crewFiles: async (endpoint, method, body) => {
      options.beforeFetch?.();
      calls.push({ endpoint, method, body });
      const response = responses[responseIndex++];
      if (!response) throw new Error('The picker harness ran out of responses.');
      return response;
    },
    isClosed: () => closed,
  };
  return {
    calls,
    dialogEvents,
    closeSender: () => {
      closed = true;
    },
    // The order `main.ts` runs them in, inside an async handler: the request is validated
    // before anything else, and a refusal is a rejected promise.
    invoke: async (raw: unknown) => selectCrewTransferFile(parseCrewPickerRequest(raw), deps),
  };
}

const jsonResponse = (value: unknown, ok = true): DaemonAnswer => ({ ok, body: value });

/** How a thrown sentence reads: its message, or `''` when nothing was thrown. */
async function refusalOf(flow: Promise<unknown>): Promise<string> {
  try {
    await flow;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
  return '';
}

describe('Crew transfer picker opaque-capability bridge', () => {
  it('keeps selected filesystem paths in the main-process request and returns only the daemon capability', async () => {
    const flow = sharePathFunction('selectCrewTransferFile');
    const returnStart = flow.lastIndexOf('return {');
    expect(flow).toMatch(/path: chosen/);
    expect(sharePathFunction('parseCrewPickerRequest')).toMatch(
      /const purpose = options\.purpose \?\? 'transfer'/
    );
    expect(flow).toMatch(/request\.purpose === 'cleanup'/);
    expect(flow).toMatch(/properties: \['openDirectory'\]/);
    expect(flow).toMatch(/typeof result\??\.capability_id !== 'string'/);
    expect(flow.slice(returnStart)).toMatch(
      /return \{\s*capability_id: result\.capability_id as string,\s*name: result\.name as string/
    );
    expect(flow.slice(returnStart)).not.toMatch(/selected|chosen|path|bytes|contents/);
    expect(flow).toMatch(/if \(!selected\) return null/);

    // And at run time: whatever else the daemon's answer carries, the renderer gets the
    // capability, the name and the size, and never a path.
    const harness = createPickerHarness(
      [
        jsonResponse({
          capability_id: 'upload-1',
          name: 'report.txt',
          size: 12,
          path: '/Users/frank/private/report.txt',
          bytes: 'cmVwb3J0',
          contents: 'report',
        }),
      ],
      { open: { canceled: false, filePaths: ['/Users/frank/private/report.txt'] } }
    );
    const result = await harness.invoke(transferRequest({ direction: 'upload' }));
    expect(result).toEqual({ capability_id: 'upload-1', name: 'report.txt', size: 12 });
    expect(harness.calls[0].body).toMatchObject({ path: '/Users/frank/private/report.txt' });
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

  it('validates the request in main.ts before any native window, and runs the flow inside holdCrewSheet', () => {
    const handler = pickerHandlerSource();
    const parse = handler.indexOf('const request = parseCrewPickerRequest(raw);');
    const hold = handler.indexOf("return holdCrewSheet(crewSheetGate, owner.id, 'picker', () =>");
    const flow = handler.indexOf('selectCrewTransferFile(request, {');
    const firstDialog = handler.search(/dialog\.show/);
    expect(parse).toBeGreaterThanOrEqual(0);
    expect(hold).toBeGreaterThan(parse);
    expect(flow).toBeGreaterThan(hold);
    expect(firstDialog).toBeGreaterThan(flow);
    // Nothing before the hold opens a window, reaches the daemon or reads the raw request again.
    const beforeHold = handler.slice(handler.indexOf('async (event'), hold);
    expect(beforeHold).not.toMatch(/dialog\.|fetch\(|selectCrewTransferFile/);
    expect(beforeHold.split('raw').length - 1).toBe(2);
    // What goes back to the renderer is the held flow's answer; the other return hands the
    // daemon's answer to the flow, never to the renderer.
    expect([...handler.matchAll(/\breturn\b[^\n]*/g)].map(([line]) => line)).toEqual([
      "return holdCrewSheet(crewSheetGate, owner.id, 'picker', () =>",
      'return { ok: response.ok, body: await response.json().catch(() => null) };',
    ]);
    // Every daemon call carries the secret and the proof, and a closed window stops the flow.
    expect(handler).toContain('fetch(`${baseUrl}/crew/files${endpoint}`');
    expect(handler).toMatch(/'X-Secret-Key': getServerSecret\(settings\)/);
    expect(handler).toMatch(/'X-User-Action': getUserActionKey\(settings\)/);
    expect(handler).toContain('isClosed: () => event.sender.isDestroyed() || owner.isDestroyed()');
  });

  it('rejects an invalid privacy enum before opening a native picker', async () => {
    const parse = sharePathFunction('parseCrewPickerRequest');
    expect(parse).toMatch(/options\.expectedMode !== undefined/);
    expect(parse).toMatch(/options\.expectedMode !== 'private'/);
    expect(parse).toMatch(/options\.expectedMode !== 'public'/);

    for (const expectedMode of ['secret', 'PRIVATE', 7, null]) {
      const harness = createPickerHarness([]);
      await expect(harness.invoke(transferRequest({ expectedMode }))).rejects.toThrow(
        'Invalid expected transfer privacy. Refresh the workspace before choosing a file.'
      );
      expect(harness.dialogEvents).toEqual([]);
      expect(harness.calls).toEqual([]);
    }
  });

  it('shows uncoded daemon text only made visible, on one line, and at most 300 characters', async () => {
    // Daemon text reaches a sentence through `daemonRefusalSentence` alone, which makes it
    // visible and bounds it; the flow never reads the daemon's `error` field itself.
    expect(sharePathFunction('selectCrewTransferFile')).not.toMatch(/\.error\b/);
    expect(sharePathFunction('selectCrewTransferFile')).toContain(
      'daemonRefusalSentence(answer.body)'
    );
    const relay = sharePathFunction('daemonRefusalSentence');
    expect(relay).toContain('visibleText(text)');
    expect(relay).toContain('DAEMON_SENTENCE_MAX_CHARS');

    const hostile = `Refused.\nFake second line \u202Eexe.pdf\u2028third ${'x'.repeat(400)}`;
    const harness = createPickerHarness([
      jsonResponse({ code: 'crew_transfer_refused', error: hostile }, false),
    ]);
    const message = await refusalOf(harness.invoke(transferRequest()));
    expect(message.startsWith('Refused.\uFFFDFake second line \uFFFDexe.pdf\uFFFDthird x')).toBe(
      true
    );
    expect(message).not.toMatch(/[\n\r\u2028\u2029\u202A-\u202E\u2066-\u2069]/u);
    expect(Array.from(message)).toHaveLength(DAEMON_SENTENCE_MAX_CHARS);
    expect(message.endsWith('…')).toBe(true);
  });

  it.each([
    ['a credential file', CREW_FILE_IS_CREDENTIAL, 'upload'],
    ['a credential location', CREW_FILE_IS_CREDENTIAL, 'download'],
    ['a leading-dot name', CREW_FILE_NAME_HIDDEN, 'download'],
    ['a shared folder', CREW_FOLDER_SHARED, 'download'],
    ['a privacy change', CREW_MODE_MISMATCH, 'upload'],
    ['a privacy change on a download', CREW_MODE_MISMATCH, 'download'],
  ])(
    'rebuilds %s refusal locally and never relays the daemon text',
    async (_label, code, direction) => {
      const daemonWords = 'DAEMON WORDS\nare never relayed \u202Etxt.exe';
      const refusal = { code, error: daemonWords, actual_mode: 'public', expected_mode: 'private' };
      const harness = createPickerHarness([jsonResponse(refusal, false)], {
        open: { canceled: false, filePaths: ['/Users/frank/.config/biorouter/secrets.yaml'] },
        save: { canceled: false, filePath: '/Users/frank/.Rprofile' },
      });
      const message = await refusalOf(harness.invoke(transferRequest({ direction })));
      const chosen = direction === 'upload' ? 'secrets.yaml' : '.Rprofile';
      expect(message).toBe(
        crewFileRefusal(refusal, direction as 'upload' | 'download', chosen, {
          expectedMode: 'private',
        })
      );
      expect(message).not.toContain('DAEMON WORDS');
      expect(message).not.toContain('\u202E');
      expect(harness.calls).toHaveLength(1);
    }
  );

  it('rebuilds a coded refusal at the confirm step too, and never relays uncoded confirm text', async () => {
    const coded = createPickerHarness(
      [
        jsonResponse({ capability_id: 'cap-4', name: 'report.txt', target_exists: false }),
        jsonResponse({ code: CREW_MODE_MISMATCH, error: 'DAEMON WORDS' }, false),
      ],
      { messages: [1] }
    );
    const codedMessage = await refusalOf(coded.invoke(transferRequest()));
    expect(codedMessage).toBe(
      'Your connection is now Public; this file was checked for Private. Refresh Crew and try again.'
    );

    const uncoded = createPickerHarness([
      jsonResponse({ capability_id: 'cap-5', name: 'report.txt', target_exists: false }),
      jsonResponse({ error: 'destination changed on the server' }, false),
    ]);
    await expect(uncoded.invoke(transferRequest())).rejects.toThrow(
      'The selected destination could not be confirmed. Choose the destination again and review any replacement request.'
    );
    expect(uncoded.calls.map((call) => call.endpoint)).toEqual(['', '/cap-5/confirm']);
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
    expect(harness.calls[0]).toMatchObject({ endpoint: '', method: 'POST' });
    expect(harness.calls[0].body).toMatchObject({
      path: '/tmp/report.txt',
      approval_pending: true,
      overwrite: true,
    });
    expect(harness.calls[1]).toEqual({ endpoint: '/cap-1/confirm', method: 'POST', body: {} });
    expect(harness.dialogEvents).toEqual(['save', 'Replace Crew download destination']);
  });

  it('refuses a confirmation that names another capability', async () => {
    const harness = createPickerHarness(
      [
        jsonResponse({ capability_id: 'cap-1', name: 'report.txt', target_exists: false }),
        jsonResponse({ capability_id: 'cap-other', name: 'report.txt' }),
      ],
      { messages: [1] }
    );
    await expect(harness.invoke(transferRequest())).rejects.toThrow(
      'The daemon did not confirm the selected destination. Choose it again.'
    );
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
    expect(harness.calls[1]).toEqual({ endpoint: '/cap-2', method: 'DELETE', body: {} });
    expect(harness.calls.some((call) => call.endpoint.endsWith('/confirm'))).toBe(false);
  });

  it('shows a stale destination refusal as the daemon worded it, made visible, and leaves upload and cleanup flows independent', async () => {
    const stale = createPickerHarness([
      jsonResponse({ error: 'destination changed on the server' }, false),
    ]);
    await expect(stale.invoke(transferRequest())).rejects.toThrow(
      'destination changed on the server'
    );
    expect(stale.dialogEvents).toEqual(['save']);

    const silent = createPickerHarness([jsonResponse(null, false)]);
    await expect(silent.invoke(transferRequest())).rejects.toThrow(
      'The daemon refused this file selection. Choose an accessible file or a new destination filename.'
    );

    const upload = createPickerHarness(
      [jsonResponse({ capability_id: 'upload-1', name: 'upload.txt' })],
      { open: { canceled: false, filePaths: ['/tmp/upload.txt'] } }
    );
    await expect(upload.invoke(transferRequest({ direction: 'upload' }))).resolves.toEqual({
      capability_id: 'upload-1',
      name: 'upload.txt',
    });
    expect(upload.dialogEvents).toEqual(['open']);
    expect(upload.calls[0].body).toMatchObject({
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
    expect(cleanup.calls[0].body).toMatchObject({
      purpose: 'cleanup',
      path: '/tmp/report.txt',
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
      { beforeFetch: () => harness.closeSender() }
    );

    await expect(harness.invoke(transferRequest())).rejects.toThrow(
      'The file selection window closed.'
    );
    expect(harness.dialogEvents).toEqual(['save']);
    expect(harness.calls).toHaveLength(1);
  });

  it('never reaches the daemon once the sender has closed', async () => {
    const harness = createPickerHarness([]);
    harness.closeSender();
    await expect(harness.invoke(transferRequest({ direction: 'upload' }))).rejects.toThrow(
      'The file selection window closed.'
    );
    expect(harness.calls).toEqual([]);
  });
});
