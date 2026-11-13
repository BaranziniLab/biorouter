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

const mocks = vi.hoisted(() => ({
  installed: [] as Array<{ name: string }>,
  success: vi.fn(),
}));

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    extensionsList: [],
    getExtensions: vi.fn(async () => mocks.installed),
  }),
}));

vi.mock('../toasts', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../toasts')>()),
  toastService: { success: mocks.success, error: vi.fn() },
}));

describe('ExtensionInstallModal', () => {
  const mockAddExtension = vi.fn();
  const mockSetView = vi.fn();

  /** The dialog's buttons, less the InfoTip's glyph, which explains rather than acts. */
  const actionButtons = () =>
    screen
      .getAllByRole('button')
      .filter((button) => button.getAttribute('data-slot') !== 'info-tip-trigger');

  const getAddExtensionEventHandler = () => {
    const addExtensionCall = mockElectron.on.mock.calls.find((call) => call[0] === 'add-extension');
    expect(addExtensionCall).toBeDefined();
    return addExtensionCall![1];
  };

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.installed = [];
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
      // The consequence stays on screen; the advice behind it is the InfoTip's.
      expect(screen.getByText(/Once installed it can read your chats/)).toBeVisible();
      expect(
        screen.getByRole('button', { name: 'About installing this extension' })
      ).toHaveAccessibleDescription(/may pose security risks/);
      // The command is data, set in the code face under a sans label.
      expect(screen.getByText('npx untrusted-extension')).toHaveClass('font-mono', 'text-code');
      expect(screen.getByText('Command')).not.toHaveClass('font-mono');
      expect(actionButtons()).toHaveLength(3);
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
      // A trusted install has no advice to fold.
      expect(screen.queryByRole('button', { name: /^About / })).toBeNull();
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
      expect(actionButtons()).toHaveLength(2);
      // The refusal's reason is visible; who to ask is the InfoTip beside it.
      expect(
        screen.getByText(/not in the allowed list, so its installation is blocked/)
      ).toBeVisible();
      expect(
        screen.getByRole('button', { name: 'About installing this extension' })
      ).toHaveAccessibleDescription(/Contact your administrator/);
    });
  });

  describe('An extension that is already installed', () => {
    it('is said once, in a toast titled with its name, and opens no dialog', async () => {
      mocks.installed = [{ name: 'TestExt' }];
      mockElectron.getAllowedExtensions.mockResolvedValue([]);

      render(<ExtensionInstallModal addExtension={mockAddExtension} setView={mockSetView} />);
      const eventHandler = getAddExtensionEventHandler();
      await act(async () => {
        await eventHandler({}, 'biorouter://extension?cmd=npx&arg=test-extension&name=TestExt');
      });

      expect(screen.queryByRole('dialog')).toBeNull();
      expect(mocks.success).toHaveBeenCalledWith({
        title: 'TestExt',
        msg: 'Already installed. Start a new chat to use it.',
      });
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
      expect(screen.getByRole('button', { name: 'Cancel' }).className).toMatch(/\bbr-fill-rest\b/);
      expect(screen.getByRole('button', { name: 'Cancel' }).className).not.toMatch(
        /border-border-emphasized/
      );
      expect(screen.getByRole('button', { name: 'Install' })).toBeInTheDocument();
    });

    it('answers a blocked install with one quiet acknowledgement', async () => {
      mockElectron.getAllowedExtensions.mockResolvedValue(['uvx allowed-package']);
      await open('biorouter://extension?cmd=npx&arg=blocked&name=Blocked');

      const gotIt = screen.getByRole('button', { name: 'Got it' });
      expect(gotIt.className).toMatch(/\bbr-fill-rest\b/);
      expect(gotIt.className).not.toMatch(/border-border-emphasized/);
    });
  });
});
