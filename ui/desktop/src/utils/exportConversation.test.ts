import { beforeEach, describe, expect, it, vi } from 'vitest';

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

import {
  EXPORT_FAILED_TOAST_TITLE,
  exportConversation,
  exportFailureMessage,
} from './exportConversation';

describe('exportConversation', () => {
  beforeEach(() => {
    mocks.exportSession.mockReset();
    mocks.toastSuccess.mockClear();
    mocks.toastError.mockClear();
    URL.createObjectURL = vi.fn(() => 'blob:chat');
    URL.revokeObjectURL = vi.fn();
  });

  it('reads the chat with the user proof and downloads it under the chat name', async () => {
    mocks.exportSession.mockResolvedValue({ data: '{"id":"s1"}' });
    const clicks: string[] = [];
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (
      this: HTMLAnchorElement
    ) {
      clicks.push(this.download);
    });

    await expect(exportConversation('s1', 'Cohort pull')).resolves.toBe(true);

    expect(mocks.exportSession).toHaveBeenCalledWith({
      path: { session_id: 's1' },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });
    expect(clicks).toEqual(['Cohort pull.json']);
    expect(mocks.toastSuccess).toHaveBeenCalledWith(
      expect.objectContaining({ title: 'Chat exported' })
    );
    click.mockRestore();
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

describe('exportFailureMessage', () => {
  it('falls back to a sentence when the failure carries no words', () => {
    expect(exportFailureMessage({})).toBe(
      'The chat could not be read for export. Nothing was downloaded.'
    );
  });
});
