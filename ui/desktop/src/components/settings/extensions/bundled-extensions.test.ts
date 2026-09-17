import { describe, expect, it, vi } from 'vitest';
import type { ExtensionConfig } from '../../../api/types.gen';
import { syncBundledExtensions } from './bundled-extensions';

describe('syncBundledExtensions', () => {
  it('installs bundled capabilities with the requested fresh-install defaults', async () => {
    const addExtension = vi.fn(
      async (_name: string, _config: ExtensionConfig, _enabled: boolean) => undefined
    );

    await syncBundledExtensions([], addExtension);

    const enabledByName = Object.fromEntries(
      addExtension.mock.calls.map(([name, _config, enabled]) => [name, enabled])
    );
    expect(enabledByName).toEqual({
      developer: true,
      computercontroller: true,
      webdocuments: true,
      autovisualiser: true,
      memory: true,
      knowledge: true,
      agent_drafter: true,
    });
  });

  it('preserves an existing disabled builtin and its restricted tool list', async () => {
    const configured = {
      type: 'builtin' as const,
      name: 'computercontroller',
      description: 'Configured computer use',
      enabled: false,
      available_tools: ['get_app_state'],
    };
    const addExtension = vi.fn(async () => undefined);
    await syncBundledExtensions([configured], addExtension);
    expect(addExtension).not.toHaveBeenCalledWith(
      'computercontroller',
      expect.anything(),
      expect.anything()
    );
    expect(configured.available_tools).toEqual(['get_app_state']);
    expect(configured.enabled).toBe(false);
  });

  it('drops a persisted Tutorial builtin during upgrade sync', async () => {
    const existingExtensions = [
      {
        type: 'builtin' as const,
        name: 'Tutorial',
        description: 'Retired tutorial capability',
        enabled: true,
        bundled: true,
      },
    ];
    const addExtension = vi.fn(
      async (_name: string, _config: ExtensionConfig, _enabled: boolean) => undefined
    );

    await syncBundledExtensions(existingExtensions, addExtension);

    expect(existingExtensions).toEqual([]);
    expect(addExtension).not.toHaveBeenCalledWith('tutorial', expect.anything(), expect.anything());
  });
});
