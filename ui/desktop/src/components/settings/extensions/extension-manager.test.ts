import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ExtensionConfig } from '../../../api/types.gen';

const mocks = vi.hoisted(() => ({
  success: vi.fn(),
  error: vi.fn(),
}));

vi.mock('../../../toasts', () => ({
  toastService: {
    success: mocks.success,
    error: mocks.error,
  },
}));

import { toggleExtensionDefault } from './extension-manager';

const capability: ExtensionConfig = {
  type: 'platform',
  name: 'skills',
  description: 'Reusable skills',
};

describe('toggleExtensionDefault', () => {
  beforeEach(() => vi.clearAllMocks());

  // The switch is the confirmation (owner decision 6.9): a flip raises no toast, in either
  // direction, for an extension or for a capability.
  it.each([
    ['on', 'toggleOn'],
    ['off', 'toggleOff'],
  ] as const)('says nothing when an extension is turned %s', async (_label, toggle) => {
    const addToConfig = vi.fn(async () => undefined);

    await toggleExtensionDefault({ toggle, extensionConfig: capability, addToConfig });

    expect(addToConfig).toHaveBeenCalledWith('skills', capability, toggle === 'toggleOn');
    expect(mocks.success).not.toHaveBeenCalled();
    expect(mocks.error).not.toHaveBeenCalled();
  });

  it('says nothing when a capability is turned on', async () => {
    const addToConfig = vi.fn(async () => undefined);

    await toggleExtensionDefault({
      toggle: 'toggleOn',
      extensionConfig: capability,
      addToConfig,
      itemKind: 'capability',
    });

    expect(mocks.success).not.toHaveBeenCalled();
  });

  // A failure is the one thing worth a toast: then the switch is what is wrong.
  it.each([
    ['extension', undefined, 'Failed to update extension default'],
    ['capability', 'capability' as const, 'Failed to update capability default'],
  ])(
    'still raises the error toast and rethrows when a %s cannot be saved',
    async (_k, itemKind, msg) => {
      const failure = new Error('disk full');
      const addToConfig = vi.fn(async () => {
        throw failure;
      });

      await expect(
        toggleExtensionDefault({
          toggle: 'toggleOn',
          extensionConfig: capability,
          addToConfig,
          itemKind,
        })
      ).rejects.toBe(failure);

      expect(mocks.error).toHaveBeenCalledWith({ title: 'skills', msg });
      expect(mocks.success).not.toHaveBeenCalled();
    }
  );
});
