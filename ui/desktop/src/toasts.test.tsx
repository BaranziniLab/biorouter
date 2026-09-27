import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  toast: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
  info: vi.fn(),
  warning: vi.fn(),
  loading: vi.fn(),
  update: vi.fn(),
  isActive: vi.fn(() => false),
}));

// react-toastify is the only thing `toastService.success` actually reaches; stub
// the whole module so the assertions are about the options we hand it.
vi.mock('react-toastify', () => ({
  toast: Object.assign(mocks.toast, {
    success: mocks.success,
    error: mocks.error,
    info: mocks.info,
    warning: mocks.warning,
    loading: mocks.loading,
    update: mocks.update,
    dismiss: vi.fn(),
    isActive: mocks.isActive,
  }),
}));

import {
  toastError,
  toastInfo,
  toastLoading,
  toastService,
  toastSuccess,
  toastWarning,
} from './toasts';

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

/**
 * T-57 (a11y P2-8). react-toastify defaults EVERY toast to `role="alert"`, an
 * assertive live region, so "@crew_x joined chen-lab" cut across whatever the
 * screen reader was reading. The role is now chosen per entry point, and each one
 * is pinned here: the options object is what reaches the DOM (`Toastify__toast`
 * renders `role={role}`), so asserting it is asserting the announced role.
 */
describe('which toasts interrupt a screen reader', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.isActive.mockReturnValue(false);
    toastService.configure({ silent: false });
  });

  const optionsOf = (fn: ReturnType<typeof vi.fn>) => fn.mock.calls[0][1];

  it('reads confirmations politely: success is a status, not an alert', () => {
    toastSuccess({ title: 'Joined', msg: '@crew_x joined chen-lab' });
    expect(optionsOf(mocks.success)).toMatchObject({ role: 'status' });

    // The service path is the same function, so it inherits the role.
    vi.clearAllMocks();
    toastService.success({ title: 'Saved', msg: 'All good' });
    expect(optionsOf(mocks.success)).toMatchObject({ role: 'status' });
  });

  it('reads information politely', () => {
    toastInfo({ title: 'Heads up', msg: 'Nothing is wrong' });
    expect(optionsOf(mocks.info)).toMatchObject({ role: 'status' });
  });

  it('reads progress politely', () => {
    toastLoading({ title: 'Installing', msg: 'One moment' });
    expect(optionsOf(mocks.loading)).toMatchObject({ role: 'status' });
  });

  // The other half of the rule, and the half that must not regress: a failure
  // exists to interrupt, so it keeps the assertive role.
  it('keeps failures assertive', () => {
    toastError({ title: 'Failed', msg: 'The chat store was busy' });
    expect(optionsOf(mocks.error)).toMatchObject({ role: 'alert' });
  });

  it('keeps warnings assertive', () => {
    toastWarning({ title: 'Careful', msg: 'This model cannot see images' });
    expect(optionsOf(mocks.warning)).toMatchObject({ role: 'alert' });
  });

  it('lets the grouped extension report interrupt only when something failed', () => {
    const loading = [{ name: 'developer', status: 'loading' as const }];
    toastService.extensionLoading(loading, 1, false);
    expect(mocks.toast.mock.calls[0][1]).toMatchObject({ role: 'status' });

    // Once the toast exists, the report is UPDATED in place; the role travels
    // with the update so a failure turns the same toast assertive.
    mocks.isActive.mockReturnValue(true);
    const failed = [{ name: 'developer', status: 'error' as const, error: 'boom' }];
    toastService.extensionLoading(failed, 1, true);
    expect(mocks.update.mock.calls[0][1]).toMatchObject({ type: 'error', role: 'alert' });

    const loaded = [{ name: 'developer', status: 'success' as const }];
    toastService.extensionLoading(loaded, 1, true);
    expect(mocks.update.mock.calls[1][1]).toMatchObject({ type: 'success', role: 'status' });
  });
});
