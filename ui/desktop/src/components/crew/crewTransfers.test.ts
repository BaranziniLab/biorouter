import { beforeEach, describe, expect, it, vi } from 'vitest';
import { beginTransfer, cancelUpload } from './crewTransfers';

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
}));

vi.mock('./crewApi', async () => {
  const actual = await vi.importActual<typeof import('./crewApi')>('./crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe('Crew transfer privacy handoff', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mocks.crewHttp.mockReset();
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: { crewSelectTransferFile: vi.fn() },
    });
  });

  it('captures the verified mode before an async picker and starts only with its opaque capability', async () => {
    const selection = deferred<{ capability_id: string; name: string }>();
    const picker = window.electron.crewSelectTransferFile as ReturnType<typeof vi.fn>;
    picker.mockReturnValue(selection.promise);
    mocks.crewHttp.mockResolvedValue({
      id: 'transfer-1',
      connection_id: 'connection-1',
      channel_id: 'channel-1',
      direction: 'upload',
    });

    const transfer = beginTransfer({
      expected_mode: 'private',
      connection_id: 'connection-1',
      channel_id: 'channel-1',
      direction: 'upload',
    });

    expect(picker).toHaveBeenCalledWith(
      expect.objectContaining({
        expectedMode: 'private',
        connectionId: 'connection-1',
        channelId: 'channel-1',
      })
    );
    selection.resolve({ capability_id: 'capability-1', name: 'notes.txt' });
    await expect(transfer).resolves.toMatchObject({ id: 'transfer-1' });
    expect(mocks.crewHttp).toHaveBeenCalledWith(
      '/transfers',
      'POST',
      expect.objectContaining({
        connection_id: 'connection-1',
        channel_id: 'channel-1',
        file_capability: 'capability-1',
      })
    );
    expect(mocks.crewHttp.mock.calls[0][2]).not.toHaveProperty('expected_mode');
  });

  it("rethrows a picker refusal as the main process's sentence, without Electron's wrapper (FILES-F6)", async () => {
    const picker = window.electron.crewSelectTransferFile as ReturnType<typeof vi.fn>;
    picker.mockRejectedValue(
      new Error(
        "Error invoking remote method 'crew:select-transfer-file': Error: Finish the open Save or Open window first."
      )
    );
    await expect(
      beginTransfer({
        connection_id: 'connection-1',
        channel_id: 'channel-1',
        direction: 'download',
        blob_id: 'blob-1',
      })
    ).rejects.toThrow(/^Finish the open Save or Open window first\.$/);
    expect(mocks.crewHttp).not.toHaveBeenCalled();
  });

  it('does not turn a cancelled async picker into a transfer request', async () => {
    const picker = window.electron.crewSelectTransferFile as ReturnType<typeof vi.fn>;
    picker.mockResolvedValue(null);

    await expect(
      beginTransfer({
        expected_mode: 'public',
        connection_id: 'connection-1',
        channel_id: 'channel-1',
        direction: 'upload',
      })
    ).resolves.toBeNull();
    expect(mocks.crewHttp).not.toHaveBeenCalled();
  });
});

describe('cancelUpload (FILES-F7)', () => {
  beforeEach(() => {
    mocks.crewHttp.mockReset();
  });
  const receipt = (state: string, direction = 'upload') => ({
    id: 'upload-1',
    direction,
    state,
  });
  const noWait = { wait: async () => undefined };

  it('pauses a moving upload, waits until it has stopped, then forgets its record', async () => {
    const states = ['uploading', 'pause_requested', 'needs_file_selection'];
    mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
      if (method === 'GET') return receipt(states.shift() ?? 'needs_file_selection');
      return {};
    });
    await expect(cancelUpload('upload-1', noWait)).resolves.toBe('cancelled');
    expect(mocks.crewHttp.mock.calls.map(([path, method]) => `${method ?? 'GET'} ${path}`)).toEqual(
      [
        'GET /transfers/upload-1',
        'POST /transfers/upload-1/pause',
        'GET /transfers/upload-1',
        'GET /transfers/upload-1',
        'DELETE /transfers/upload-1',
      ]
    );
  });

  it('forgets a paused upload at once, without pausing it again', async () => {
    mocks.crewHttp.mockImplementation(async (_path: string, method = 'GET') =>
      method === 'GET' ? receipt('needs_file_selection') : {}
    );
    await expect(cancelUpload('upload-1', noWait)).resolves.toBe('cancelled');
    expect(mocks.crewHttp).toHaveBeenCalledTimes(2);
    expect(mocks.crewHttp).toHaveBeenLastCalledWith('/transfers/upload-1', 'DELETE');
  });

  it('leaves an upload that finished before it could stop, which is a whole file now', async () => {
    const states = ['publishing', 'completed'];
    mocks.crewHttp.mockImplementation(async (_path: string, method = 'GET') =>
      method === 'GET' ? receipt(states.shift() ?? 'completed') : {}
    );
    await expect(cancelUpload('upload-1', noWait)).resolves.toBe('finished');
    expect(mocks.crewHttp).not.toHaveBeenCalledWith('/transfers/upload-1', 'DELETE');
  });

  it('gives up in words when the upload does not stop, and forgets nothing', async () => {
    mocks.crewHttp.mockImplementation(async (_path: string, method = 'GET') =>
      method === 'GET' ? receipt('pause_requested') : {}
    );
    await expect(cancelUpload('upload-1', { ...noWait, attempts: 3 })).rejects.toThrow(
      'Crew couldn’t stop that upload yet. Try again in a moment.'
    );
    expect(mocks.crewHttp).not.toHaveBeenCalledWith('/transfers/upload-1', 'DELETE');
  });

  it('never cancels a download this way', async () => {
    mocks.crewHttp.mockResolvedValue(receipt('downloading', 'download'));
    await expect(cancelUpload('upload-1', noWait)).rejects.toThrow(
      'Only an upload can be cancelled.'
    );
    expect(mocks.crewHttp).toHaveBeenCalledTimes(1);
  });
});
