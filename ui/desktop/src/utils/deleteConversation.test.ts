import { beforeEach, describe, expect, it, vi } from 'vitest';
import { deleteConversation } from './deleteConversation';
const mocks = vi.hoisted(() => ({
  api: vi.fn(),
  headers: vi.fn(),
  update: vi.fn(),
  notify: vi.fn(),
}));
vi.mock('../api', () => ({ deleteSession: mocks.api }));
vi.mock('./userAction', () => ({ userActionHeaders: mocks.headers }));
vi.mock('./sessionListCache', () => ({
  updateCachedSessionList: mocks.update,
  notifySessionListChanged: mocks.notify,
}));
beforeEach(() => {
  vi.clearAllMocks();
  mocks.headers.mockResolvedValue({ 'X-User-Action': 'fixture' });
});
describe('permanent conversation cleanup', () => {
  it('authorizes deletion, then removes only its cache entry and notifies open tabs', async () => {
    mocks.api.mockResolvedValue({});
    await deleteConversation('disposable');
    expect(mocks.api).toHaveBeenCalledWith({
      path: { session_id: 'disposable' },
      headers: { 'X-User-Action': 'fixture' },
      throwOnError: true,
    });
    expect(mocks.update.mock.calls[0][0]([{ id: 'disposable' }, { id: 'keep' }])).toEqual([
      { id: 'keep' },
    ]);
    expect(mocks.notify).toHaveBeenCalledWith({ removed: 'disposable' });
  });
  it('leaves history and open tabs intact when deletion fails', async () => {
    mocks.api.mockRejectedValue(new Error('unavailable'));
    await expect(deleteConversation('disposable')).rejects.toThrow('unavailable');
    expect(mocks.update).not.toHaveBeenCalled();
    expect(mocks.notify).not.toHaveBeenCalled();
  });
});
