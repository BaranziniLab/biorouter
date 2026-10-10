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
  dismissScreenToasts,
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

  it('lets the grouped extension report interrupt only when something failed', async () => {
    const loading = [{ name: 'developer', status: 'loading' as const }];
    toastService.extensionLoading(loading, 1, false);
    expect(mocks.toast.mock.calls[0][1]).toMatchObject({ role: 'status' });

    // Once the toast exists, the report is UPDATED in place; the role travels
    // with the update so a failure turns the same toast assertive.
    mocks.isActive.mockReturnValue(true);
    const failed = [{ name: 'developer', status: 'error' as const, error: 'boom' }];
    toastService.extensionLoading(failed, 1, true);
    expect(mocks.update.mock.calls[0][1]).toMatchObject({ type: 'error', role: 'alert' });

    // A clean load arriving now does not overwrite the failure report.
    const { toast } = await import('react-toastify');
    const loaded = [{ name: 'developer', status: 'success' as const }];
    toastService.extensionLoading(loaded, 1, true);
    expect(mocks.update).toHaveBeenCalledTimes(1);
    expect(toast.dismiss).not.toHaveBeenCalled();
  });
});

// Vision V2-14: "All extensions loaded" landed over the docked summary rail on a
// new chat's first turn, reporting a load the person did not ask for.
describe('a clean extension load raises no toast', () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    mocks.isActive.mockReturnValue(false);
    toastService.configure({ silent: false });
    const { toast } = await import('react-toastify');
    (toast.dismiss as ReturnType<typeof vi.fn>).mockClear();
  });

  it('raises nothing when every extension loaded', () => {
    const loaded = [
      { name: 'developer', status: 'success' as const },
      { name: 'memory', status: 'success' as const },
    ];
    toastService.extensionLoading(loaded, 2, true);
    expect(mocks.toast).not.toHaveBeenCalled();
    expect(mocks.update).not.toHaveBeenCalled();
  });

  it('takes down a progress report that ends cleanly', async () => {
    const { toast } = await import('react-toastify');
    toastService.extensionLoading([{ name: 'developer', status: 'loading' as const }], 1, false);
    expect(mocks.toast).toHaveBeenCalledTimes(1);

    mocks.isActive.mockReturnValue(true);
    toastService.extensionLoading([{ name: 'developer', status: 'success' as const }], 1, true);
    expect(mocks.update).not.toHaveBeenCalled();
    expect(toast.dismiss).toHaveBeenCalledWith('extension-loading');
  });

  it('still reports a failure, assertively and until dismissed', () => {
    toastService.extensionLoading(
      [{ name: 'developer', status: 'error' as const, error: 'boom' }],
      1,
      true
    );
    expect(mocks.toast.mock.calls[0][1]).toMatchObject({
      type: 'error',
      role: 'alert',
      autoClose: false,
    });
  });
});

// W2-PRV-16: "errors persist" also meant "errors follow you". A toast about the
// screen it was raised on is dismissed when that screen goes; an app-level
// toast, error or not, is left alone.
describe('screen-scoped toasts', () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    const { toast } = await import('react-toastify');
    (toast.dismiss as ReturnType<typeof vi.fn>).mockClear();
  });

  it('dismisses only the toasts raised for the screen being left', async () => {
    const { toast } = await import('react-toastify');
    mocks.error.mockReturnValueOnce('screen-error').mockReturnValueOnce('app-error');
    mocks.warning.mockReturnValueOnce('screen-warning');

    toastError({ title: 'Diverge failed', msg: 'boom', scope: 'screen' });
    toastError({ title: 'Extension failed', msg: 'boom' });
    toastWarning({ title: 'Message not sent', msg: 'long', scope: 'screen' });

    dismissScreenToasts();
    const dismissed = (toast.dismiss as ReturnType<typeof vi.fn>).mock.calls.map((c) => c[0]);
    expect(dismissed).toEqual(['screen-error', 'screen-warning']);

    // Once dismissed, they are forgotten: the next screen change dismisses nothing.
    dismissScreenToasts();
    expect(toast.dismiss).toHaveBeenCalledTimes(2);
  });

  it('keeps errors persistent whatever their scope', () => {
    toastError({ title: 'Diverge failed', msg: 'boom', scope: 'screen' });
    expect(mocks.error.mock.calls[0][1]).toMatchObject({ autoClose: false });
  });

  it('lets a warning show its whole sentence', () => {
    toastWarning({ title: 'Message not sent', msg: 'a long sentence', clampMessage: false });
    const body = mocks.warning.mock.calls[0][0] as { props: { clampMessage?: boolean } };
    expect(body.props.clampMessage).toBe(false);

    toastWarning({ title: 'Careful', msg: 'short' });
    const clamped = mocks.warning.mock.calls[1][0] as { props: { clampMessage?: boolean } };
    expect(clamped.props.clampMessage).toBe(true);
  });
});
