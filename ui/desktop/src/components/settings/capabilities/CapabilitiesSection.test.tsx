import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CapabilitiesSection } from './CapabilitiesSection';

const mocks = vi.hoisted(() => ({
  extensionsList: [] as Array<Record<string, unknown>>,
  addExtension: vi.fn(async () => undefined),
  getExtensions: vi.fn(async () => []),
  toggleExtensionDefault: vi.fn(async () => undefined),
}));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    extensionsList: mocks.extensionsList,
    addExtension: mocks.addExtension,
    getExtensions: mocks.getExtensions,
  }),
}));

vi.mock('../extensions', () => ({
  toggleExtensionDefault: mocks.toggleExtensionDefault,
}));

describe('CapabilitiesSection', () => {
  beforeEach(() => {
    mocks.extensionsList = [];
    vi.clearAllMocks();
  });

  it('shows all capabilities and uses their declared defaults while config loads', () => {
    render(<CapabilitiesSection />);

    expect(screen.getAllByRole('switch')).toHaveLength(13);
    expect(screen.getByRole('switch', { name: 'Toggle Auto Visualiser capability' })).toBeChecked();
    expect(screen.getByRole('switch', { name: 'Toggle Code Execution capability' })).toBeChecked();
    expect(
      screen.getByRole('switch', { name: 'Toggle Biorouter Copilot capability' })
    ).toBeChecked();
    expect(screen.getByRole('switch', { name: 'Toggle Agent Drafter capability' })).toBeChecked();
    expect(screen.getByRole('switch', { name: 'Toggle Chat Recall capability' })).not.toBeChecked();
  });

  it('toggles the persisted capability state through the shared extension flow', async () => {
    mocks.extensionsList = [
      {
        type: 'platform',
        name: 'chatrecall',
        description: 'Recall chats',
        enabled: false,
      },
    ];
    render(<CapabilitiesSection />);

    fireEvent.click(screen.getByRole('switch', { name: 'Toggle Chat Recall capability' }));

    await waitFor(() =>
      expect(mocks.toggleExtensionDefault).toHaveBeenCalledWith(
        expect.objectContaining({
          toggle: 'toggleOn',
          extensionConfig: expect.objectContaining({ name: 'chatrecall', enabled: false }),
          addToConfig: mocks.addExtension,
          itemKind: 'capability',
        })
      )
    );
    expect(mocks.getExtensions).toHaveBeenCalledWith(true);
  });
});
