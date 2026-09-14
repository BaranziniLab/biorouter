import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  success: vi.fn(),
  error: vi.fn(),
}));

// react-toastify is the only thing `toastService.success` actually reaches; stub
// the whole module so the assertions are about the options we hand it.
vi.mock('react-toastify', () => ({
  toast: Object.assign(vi.fn(), {
    success: mocks.success,
    error: mocks.error,
    info: vi.fn(),
    warning: vi.fn(),
    loading: vi.fn(),
    update: vi.fn(),
    dismiss: vi.fn(),
    isActive: vi.fn(() => false),
  }),
}));

import { toastError, toastService } from './toasts';

describe('toastService.success', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    toastService.configure({ silent: false });
  });

  // ONE dwell time across the tier (Astryx §3.7): 5s for everything that
  // expires. Four different numbers used to mean four different notifications
  // felt like four different systems.
  it('keeps the shared 5s auto-close by default', () => {
    toastService.success({ title: 'Saved', msg: 'All good' });

    expect(mocks.success).toHaveBeenCalledTimes(1);
    expect(mocks.success.mock.calls[0][1]).toMatchObject({ autoClose: 5000 });
  });

  // Dedup by key is policy, not an exception: a content-identical toast fired
  // twice coalesces onto one id instead of stacking.
  it('derives a content dedup key so identical toasts coalesce', () => {
    toastService.success({ title: 'Saved', msg: 'All good' });
    toastService.success({ title: 'Saved', msg: 'All good' });

    expect(mocks.success.mock.calls[0][1].toastId).toBe(mocks.success.mock.calls[1][1].toastId);
    expect(mocks.success.mock.calls[0][1].toastId).not.toBe(undefined);
  });

  // BR-71 §3.2 (decision 14): the chatrecall suggestion is shown exactly once in
  // the lifetime of an install, so it must not be able to expire unread. That is
  // only possible if a caller can override the shared default per toast.
  it('forwards per-toast options so a caller can opt out of auto-close', () => {
    toastService.success(
      { title: 'Workspace Control enabled', msg: 'long copy' },
      {
        autoClose: false,
      }
    );

    expect(mocks.success).toHaveBeenCalledTimes(1);
    expect(mocks.success.mock.calls[0][1]).toMatchObject({ autoClose: false });
  });
});

describe('toastError', () => {
  beforeEach(() => vi.clearAllMocks());

  // D3a's second round: two chats that failed with the same sentence shared one
  // toast, so the first chat whose failure was overturned dismissed the other's
  // report. A scope keeps two subjects apart without giving up dedup within one.
  it('a dedupe scope separates identical failures about different subjects', () => {
    const busy = { title: 'The chat store was busy', msg: 'Try again in a moment.' };
    toastError({ ...busy, dedupeScope: 'declassify:a' });
    toastError({ ...busy, dedupeScope: 'declassify:b' });
    toastError({ ...busy, dedupeScope: 'declassify:a' });
    toastError(busy);

    const ids = mocks.error.mock.calls.map((call) => call[1].toastId);
    expect(ids[0]).not.toBe(ids[1]);
    expect(ids[2]).toBe(ids[0]);
    // Unscoped callers keep the content key they always had.
    expect(ids[3]).toBe('error:The chat store was busy:Try again in a moment.');
    expect(ids[3]).not.toBe(ids[0]);
  });
});
