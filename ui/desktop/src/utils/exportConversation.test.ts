import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  exportSession: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('../api', () => ({ exportSession: mocks.exportSession }));
vi.mock('../toasts', () => ({ toastSuccess: mocks.toastSuccess, toastError: mocks.toastError }));
vi.mock('./userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

import { chatRowCopy } from '../components/chats/copy';
import {
  EXPORT_FAILED_TOAST_TITLE,
  chatExportFileName,
  exportConversation,
  exportFailureMessage,
  savedFileParts,
} from './exportConversation';
import { BROWSER_SURFACE_MARKER } from './surface';

const copy = chatRowCopy.export;

describe('exportConversation', () => {
  const showSaveDialog = vi.fn();
  const writeFile = vi.fn();
  let anchorClicks: string[];

  beforeEach(() => {
    mocks.exportSession.mockReset();
    mocks.toastSuccess.mockClear();
    mocks.toastError.mockClear();
    showSaveDialog.mockReset();
    writeFile.mockReset();
    URL.createObjectURL = vi.fn(() => 'blob:chat');
    URL.revokeObjectURL = vi.fn();
    anchorClicks = [];
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (
      this: HTMLAnchorElement
    ) {
      anchorClicks.push(this.download);
    });
    Object.assign(window, { electron: { showSaveDialog, writeFile } });
    mocks.exportSession.mockResolvedValue({ data: '{"id":"s1"}' });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    delete document.documentElement.dataset.biorouterSurface;
  });

  it('reads the chat with the user proof, saves it where the person chose, then says where', async () => {
    showSaveDialog.mockResolvedValue({
      canceled: false,
      filePath: '/Users/me/Downloads/Cohort pull.json',
    });
    writeFile.mockResolvedValue(true);

    await expect(exportConversation('s1', 'Cohort pull')).resolves.toBe(true);

    expect(mocks.exportSession).toHaveBeenCalledWith({
      path: { session_id: 's1' },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });
    expect(showSaveDialog).toHaveBeenCalledWith({
      title: copy.dialogTitle,
      defaultPath: 'Cohort pull.json',
      filters: [{ name: copy.fileType, extensions: ['json'] }],
    });
    expect(writeFile).toHaveBeenCalledWith('/Users/me/Downloads/Cohort pull.json', '{"id":"s1"}');
    expect(mocks.toastSuccess).toHaveBeenCalledWith({
      title: copy.saved,
      msg: 'Saved "Cohort pull.json" to Downloads.',
    });
    expect(anchorClicks, 'no browser download in the app').toEqual([]);
  });

  /**
   * Vision V4-R5-2: the toast said "was downloaded" while the Save window was still open, and
   * stayed after Cancel. Nothing is announced until the file is written, and a cancel says nothing.
   */
  it('announces nothing until the file is written', async () => {
    let choose: (value: { canceled: boolean; filePath?: string }) => void = () => {};
    showSaveDialog.mockReturnValue(new Promise((resolve) => (choose = resolve)));
    writeFile.mockResolvedValue(true);

    const done = exportConversation('s1', 'Cohort pull');
    await vi.waitFor(() => expect(showSaveDialog).toHaveBeenCalled());
    expect(mocks.toastSuccess, 'nothing while the Save window is open').not.toHaveBeenCalled();

    choose({ canceled: true });
    await expect(done).resolves.toBe(false);
    expect(writeFile).not.toHaveBeenCalled();
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
    expect(mocks.toastError).not.toHaveBeenCalled();
  });

  it('says the file was not saved when the write fails', async () => {
    showSaveDialog.mockResolvedValue({ canceled: false, filePath: '/Volumes/Gone/Cohort.json' });
    writeFile.mockResolvedValue(false);

    await expect(exportConversation('s1', 'Cohort', { scope: 'screen' })).resolves.toBe(false);

    expect(mocks.toastError).toHaveBeenCalledWith({
      title: EXPORT_FAILED_TOAST_TITLE,
      msg: copy.writeFailed('Gone'),
      scope: 'screen',
    });
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
  });

  it('hands the file to the browser as a download on a served page, and claims nothing', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;

    await expect(exportConversation('s1', 'Cohort pull')).resolves.toBe(true);

    expect(anchorClicks).toEqual(['Cohort pull.json']);
    expect(showSaveDialog, 'the shim would save on the server').not.toHaveBeenCalled();
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
  });

  it("shows the daemon's sentence when the export is refused", async () => {
    const refusal = 'Crew context cannot be exported without its channel permissions.';
    mocks.exportSession.mockRejectedValue(refusal);
    vi.spyOn(console, 'error').mockImplementation(() => {});

    await expect(exportConversation('s1', 'Cohort pull', { scope: 'screen' })).resolves.toBe(false);

    expect(mocks.toastError).toHaveBeenCalledWith({
      title: EXPORT_FAILED_TOAST_TITLE,
      msg: refusal,
      scope: 'screen',
    });
    expect(showSaveDialog, 'no Save window for a refused read').not.toHaveBeenCalled();
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
  });

  it('keeps an app-scoped toast when asked', async () => {
    mocks.exportSession.mockRejectedValue(new Error('offline'));
    vi.spyOn(console, 'error').mockImplementation(() => {});
    await exportConversation('s1', 'Cohort pull', { scope: 'app' });
    expect(mocks.toastError).toHaveBeenCalledWith({
      title: EXPORT_FAILED_TOAST_TITLE,
      msg: 'offline',
    });
  });
});

describe('chatExportFileName', () => {
  it('keeps a plain name and adds .json', () => {
    expect(chatExportFileName('Cohort pull')).toBe('Cohort pull.json');
  });

  it('makes one safe file name of any chat name', () => {
    // A slash would name a folder in the Save window; the rest are refused on Windows.
    expect(chatExportFileName('Scheduled job: a/b\\c *?"<>|')).toBe(
      'Scheduled job- a-b-c ------.json'
    );
    expect(chatExportFileName('..hidden')).toBe('hidden.json');
    expect(chatExportFileName('  line\none  ')).toBe('lineone.json');
    expect(chatExportFileName('   ')).toBe(`${copy.untitled}.json`);
    expect(chatExportFileName('x'.repeat(300))).toBe(`${'x'.repeat(120)}.json`);
  });
});

describe('savedFileParts', () => {
  it('names the file and its folder', () => {
    expect(savedFileParts('/Users/me/Downloads/Cohort.json')).toEqual({
      file: 'Cohort.json',
      folder: 'Downloads',
    });
    expect(savedFileParts('C:\\Users\\me\\Desktop\\Cohort.json')).toEqual({
      file: 'Cohort.json',
      folder: 'Desktop',
    });
  });
});

describe('exportFailureMessage', () => {
  it('falls back to a sentence when the failure carries no words', () => {
    expect(exportFailureMessage({})).toBe(
      'The chat could not be read for export. Nothing was downloaded.'
    );
  });
});
