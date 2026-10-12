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

import { activateExtensionDefault, toggleExtensionDefault } from './extension-manager';

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
    ['extension', undefined, 'toggleOn', 'Could not turn on'],
    ['extension', undefined, 'toggleOff', 'Could not turn off'],
    ['capability', 'capability' as const, 'toggleOn', 'Could not turn on'],
  ] as const)(
    'still raises the error toast and rethrows when a %s cannot be saved (%s)',
    async (_k, itemKind, toggle, msg) => {
      const failure = new Error('disk full');
      const addToConfig = vi.fn(async () => {
        throw failure;
      });

      await expect(
        toggleExtensionDefault({
          toggle,
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

describe('activateExtensionDefault', () => {
  beforeEach(() => vi.clearAllMocks());

  it('confirms an added extension in the words the .brxt install uses, naming it once', async () => {
    const addToConfig = vi.fn(async () => undefined);

    await activateExtensionDefault({ addToConfig, extensionConfig: capability });

    expect(addToConfig).toHaveBeenCalledWith('skills', capability, true);
    expect(mocks.success).toHaveBeenCalledWith({
      title: 'skills',
      msg: 'Extension added and enabled',
    });
  });

  it('raises the error toast and rethrows when it cannot be added', async () => {
    const failure = new Error('disk full');
    const addToConfig = vi.fn(async () => {
      throw failure;
    });

    await expect(
      activateExtensionDefault({ addToConfig, extensionConfig: capability })
    ).rejects.toBe(failure);

    expect(mocks.error).toHaveBeenCalledWith({ title: 'skills', msg: 'Failed to add extension' });
    expect(mocks.success).not.toHaveBeenCalled();
  });
});
