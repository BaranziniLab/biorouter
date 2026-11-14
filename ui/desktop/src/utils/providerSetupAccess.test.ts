import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BROWSER_SURFACE_MARKER } from './surface';
import {
  loadProviderSetupAccess,
  providerSetupRefusal,
  providersAreHostManaged,
  resetProviderSetupAccessForTests,
  useProvidersAreHostManaged,
} from './providerSetupAccess';

/**
 * SD-18: the page's one answer to "may this browser set up providers?". Three properties a
 * rewrite gets wrong: the desktop never asks, a browser is host-managed until the daemon says
 * otherwise (and whenever it could not), and only a real answer is kept.
 */

const mocks = vi.hoisted(() => ({ getProviderSetupAccess: vi.fn() }));

vi.mock('../api', () => ({ getProviderSetupAccess: mocks.getProviderSetupAccess }));

const browser = () => {
  document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
};

describe('providerSetupAccess', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetProviderSetupAccessForTests();
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
  });

  afterEach(() => {
    delete document.documentElement.dataset.biorouterSurface;
    resetProviderSetupAccessForTests();
    vi.restoreAllMocks();
  });

  it('never asks on the desktop, where nothing is managed on a host', async () => {
    expect(providersAreHostManaged()).toBe(false);
    await expect(loadProviderSetupAccess()).resolves.toEqual({ allowed: true });
    expect(mocks.getProviderSetupAccess).not.toHaveBeenCalled();
    expect(providerSetupRefusal()).toBeUndefined();
  });

  it('is host-managed on a browser until the daemon has answered', () => {
    browser();
    expect(providersAreHostManaged()).toBe(true);
    expect(providerSetupRefusal()).toBeUndefined();
  });

  it('opens the controls once the daemon allows setup, asking once per page', async () => {
    browser();
    mocks.getProviderSetupAccess.mockResolvedValue({ data: { allowed: true } });

    await loadProviderSetupAccess();
    await loadProviderSetupAccess();

    expect(providersAreHostManaged()).toBe(false);
    expect(providerSetupRefusal()).toBeUndefined();
    expect(mocks.getProviderSetupAccess).toHaveBeenCalledTimes(1);
    expect(mocks.getProviderSetupAccess).toHaveBeenCalledWith({ throwOnError: true });
  });

  it('keeps the sentence the daemon gave when it refuses', async () => {
    browser();
    mocks.getProviderSetupAccess.mockResolvedValue({
      data: { allowed: false, reason: '  This daemon holds a key, so only the app can choose.  ' },
    });

    await expect(loadProviderSetupAccess()).resolves.toEqual({
      allowed: false,
      reason: 'This daemon holds a key, so only the app can choose.',
    });
    expect(providersAreHostManaged()).toBe(true);
    expect(providerSetupRefusal()).toBe('This daemon holds a key, so only the app can choose.');
  });

  /**
   * Fail closed, and cache the verdict rather than the error: one dropped request keeps the
   * page host-managed for now, and the next explicit load (the launch check runs one) asks again.
   */
  it('stays host-managed when the read fails, and asks again on the next load', async () => {
    browser();
    mocks.getProviderSetupAccess.mockRejectedValueOnce(new Error('offline'));

    await expect(loadProviderSetupAccess()).resolves.toEqual({ allowed: false });
    expect(providersAreHostManaged()).toBe(true);
    expect(providerSetupRefusal()).toBeUndefined();

    mocks.getProviderSetupAccess.mockResolvedValueOnce({ data: { allowed: true } });
    await loadProviderSetupAccess();
    expect(providersAreHostManaged()).toBe(false);
    expect(mocks.getProviderSetupAccess).toHaveBeenCalledTimes(2);
  });

  it('shares one request between callers that ask at once', async () => {
    browser();
    let answer: (value: unknown) => void = () => undefined;
    mocks.getProviderSetupAccess.mockReturnValue(new Promise((resolve) => (answer = resolve)));

    const first = loadProviderSetupAccess();
    const second = loadProviderSetupAccess();
    answer({ data: { allowed: true } });

    await expect(first).resolves.toEqual({ allowed: true });
    await expect(second).resolves.toEqual({ allowed: true });
    expect(mocks.getProviderSetupAccess).toHaveBeenCalledTimes(1);
  });

  it('re-renders a control when the answer arrives, without asking itself', async () => {
    browser();
    mocks.getProviderSetupAccess.mockResolvedValue({ data: { allowed: true } });

    const { result } = renderHook(() => useProvidersAreHostManaged());
    expect(result.current).toBe(true);
    expect(mocks.getProviderSetupAccess).not.toHaveBeenCalled();

    await act(async () => {
      await loadProviderSetupAccess();
    });
    expect(result.current).toBe(false);
  });
});
