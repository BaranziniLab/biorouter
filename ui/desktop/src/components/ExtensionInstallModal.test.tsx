/* eslint-disable @typescript-eslint/no-explicit-any */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import { ExtensionInstallModal } from './ExtensionInstallModal';
import { addExtensionFromDeepLink } from './settings/extensions/deeplink';

vi.mock('./settings/extensions/deeplink', () => ({
  addExtensionFromDeepLink: vi.fn(),
}));

const mockElectron = {
  getConfig: vi.fn(),
  getAllowedExtensions: vi.fn(),
  logInfo: vi.fn(),
  on: vi.fn(),
  off: vi.fn(),
};

(window as any).electron = mockElectron;

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    extensionsList: [],
    getExtensions: vi.fn().mockResolvedValue([]),
  }),
}));

describe('ExtensionInstallModal', () => {
  const mockAddExtension = vi.fn();
  const mockSetView = vi.fn();

  const getAddExtensionEventHandler = () => {
    const addExtensionCall = mockElectron.on.mock.calls.find((call) => call[0] === 'add-extension');
    expect(addExtensionCall).toBeDefined();
    return addExtensionCall![1];
  };

  beforeEach(() => {
    vi.clearAllMocks();
    mockElectron.getConfig.mockReturnValue({
      BIOROUTER_ALLOWLIST_WARNING: false,
    });
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  describe('Extension Request Handling', () => {
    it('should handle trusted extension (default behaviour, no allowlist)', async () => {
      mockElectron.getAllowedExtensions.mockResolvedValue([]);

      render(<ExtensionInstallModal addExtension={mockAddExtension} setView={mockSetView} />);

      const eventHandler = getAddExtensionEventHandler();

      await act(async () => {
        await eventHandler({}, 'biorouter://extension?cmd=npx&arg=test-extension&name=TestExt');
      });

      expect(screen.getByRole('dialog')).toBeInTheDocument();
      expect(screen.getByText('Install TestExt?')).toBeInTheDocument();
      expect(screen.getByText(/npx test-extension/)).toBeInTheDocument();
      expect(screen.getAllByRole('button')).toHaveLength(3);
    });

    it('should handle trusted extension (from allowlist)', async () => {
      mockElectron.getAllowedExtensions.mockResolvedValue(['npx test-extension']);

      render(<ExtensionInstallModal addExtension={mockAddExtension} setView={mockSetView} />);

      const eventHandler = getAddExtensionEventHandler();

      await act(async () => {
        await eventHandler({}, 'biorouter://extension?cmd=npx&arg=test-extension&name=AllowedExt');
      });

      expect(screen.getByText('Install AllowedExt?')).toBeInTheDocument();
      expect(screen.getAllByRole('button')).toHaveLength(3);
    });

    it('should handle warning mode', async () => {
      mockElectron.getConfig.mockReturnValue({
        BIOROUTER_ALLOWLIST_WARNING: true,
      });
      mockElectron.getAllowedExtensions.mockResolvedValue(['uvx allowed-package']);

      render(<ExtensionInstallModal addExtension={mockAddExtension} setView={mockSetView} />);

      const eventHandler = getAddExtensionEventHandler();

      await act(async () => {
        await eventHandler(
          {},
          'biorouter://extension?cmd=npx&arg=untrusted-extension&name=UntrustedExt'
        );
      });

      expect(screen.getByText('Install untrusted extension UntrustedExt?')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Install anyway' })).toBeInTheDocument();
      expect(screen.getAllByRole('button')).toHaveLength(3);
    });

    it('should handle i-ching-mcp-server as allowed command', async () => {
      mockElectron.getAllowedExtensions.mockResolvedValue([]);

      render(<ExtensionInstallModal addExtension={mockAddExtension} setView={mockSetView} />);

      const eventHandler = getAddExtensionEventHandler();

      await act(async () => {
        await eventHandler(
          {},
          'biorouter://extension?cmd=i-ching-mcp-server&id=i-ching&name=I%20Ching&description=I%20Ching%20divination'
        );
      });

      expect(screen.getByRole('dialog')).toBeInTheDocument();
      expect(screen.getByText('Install I Ching?')).toBeInTheDocument();
      expect(screen.getByText(/i-ching-mcp-server/)).toBeInTheDocument();
      expect(screen.getAllByRole('button')).toHaveLength(3);
    });
    it('should handle blocked extension', async () => {
      mockElectron.getAllowedExtensions.mockResolvedValue(['uvx allowed-package']);

      render(<ExtensionInstallModal addExtension={mockAddExtension} setView={mockSetView} />);

      const eventHandler = getAddExtensionEventHandler();

      await act(async () => {
        await eventHandler(
          {},
          'biorouter://extension?cmd=npx&arg=blocked-extension&name=BlockedExt'
        );
      });

      expect(screen.getByText('Cannot install BlockedExt')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Got it' })).toBeInTheDocument();
      expect(screen.getAllByRole('button')).toHaveLength(2);
      expect(screen.getByText(/Contact your administrator/)).toBeInTheDocument();
    });
  });

  describe('Modal Actions', () => {
    it('should dismiss modal correctly', async () => {
      mockElectron.getAllowedExtensions.mockResolvedValue([]);

      render(<ExtensionInstallModal addExtension={mockAddExtension} setView={mockSetView} />);

      const eventHandler = getAddExtensionEventHandler();

      await act(async () => {
        await eventHandler({}, 'biorouter://extension?cmd=npx&arg=test&name=Test');
      });

      expect(screen.getByRole('dialog')).toBeInTheDocument();

      await act(async () => {
        screen.getByRole('button', { name: 'Cancel' }).click();
      });

      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });

    it('should handle successful extension installation', async () => {
      vi.mocked(addExtensionFromDeepLink).mockResolvedValue(undefined);
      mockElectron.getAllowedExtensions.mockResolvedValue([]);

      render(<ExtensionInstallModal addExtension={mockAddExtension} setView={mockSetView} />);

      const eventHandler = getAddExtensionEventHandler();

      await act(async () => {
        await eventHandler({}, 'biorouter://extension?cmd=npx&arg=test&name=Test');
      });

      await act(async () => {
        screen.getByRole('button', { name: 'Install' }).click();
      });

      expect(addExtensionFromDeepLink).toHaveBeenCalledWith(
        'biorouter://extension?cmd=npx&arg=test&name=Test',
        mockAddExtension,
        expect.any(Function)
      );

      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });
  });

  describe('Chrome (spec 3.11)', () => {
    const open = async (link: string) => {
      render(<ExtensionInstallModal addExtension={mockAddExtension} setView={mockSetView} />);
      await act(async () => {
        await getAddExtensionEventHandler()({}, link);
      });
    };

    it('is a confirmation: width S, Cancel quiet, the verb on the primary button', async () => {
      mockElectron.getAllowedExtensions.mockResolvedValue([]);
      await open('biorouter://extension?cmd=npx&arg=test&name=Test');

      expect(screen.getByRole('dialog')).toHaveAttribute('data-size', 'sm');
      // `secondary`, not the `outline` the rest of the app has retired for a dismissal.
      expect(screen.getByRole('button', { name: 'Cancel' }).className).toMatch(
        /bg-background-medium/
      );
      expect(screen.getByRole('button', { name: 'Cancel' }).className).not.toMatch(
        /border-border-emphasized/
      );
      expect(screen.getByRole('button', { name: 'Install' })).toBeInTheDocument();
    });

    it('answers a blocked install with one quiet acknowledgement', async () => {
      mockElectron.getAllowedExtensions.mockResolvedValue(['uvx allowed-package']);
      await open('biorouter://extension?cmd=npx&arg=blocked&name=Blocked');

      const gotIt = screen.getByRole('button', { name: 'Got it' });
      expect(gotIt.className).toMatch(/bg-background-medium/);
      expect(gotIt.className).not.toMatch(/border-border-emphasized/);
    });
  });
});
