import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../toasts', async (original) => ({
  ...(await original<typeof import('../../toasts')>()),
  toastError: vi.fn(),
}));

import { ThemeProvider } from '../../contexts/ThemeContext';
import { toastError } from '../../toasts';
import ArtifactViewer from './ArtifactViewer';

/**
 * "Open in default browser" on a live app's link used to do nothing at all: the main process
 * refused the literal IP and only logged it. It now opens the app through a fresh launch link,
 * and when that fails the panel says why.
 */
function installElectronMock(openExternal: ReturnType<typeof vi.fn>) {
  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: {
      openExternal,
      embeddedBrowser: { isManagedAppUrl: vi.fn(async () => false) },
    },
  });
}

afterEach(() => vi.clearAllMocks());

describe('opening a live app’s link in the browser from the panel', () => {
  const appLink = 'http://127.0.0.1:51851/apps/cohort-explorer/';

  function renderAppLink() {
    render(
      <ThemeProvider>
        <ArtifactViewer
          artifact={{ kind: 'externalUrl', title: 'Cohort explorer', url: appLink }}
          onClose={vi.fn()}
          onOpenArtifact={vi.fn()}
        />
      </ThemeProvider>
    );
  }

  it('shows the main process’s reason when the app could not be opened', async () => {
    const openExternal = vi.fn(async () => ({
      ok: false,
      message: 'This app no longer exists.',
    }));
    installElectronMock(openExternal);
    renderAppLink();
    await userEvent.click(await screen.findByRole('button', { name: /open in default browser/i }));
    expect(openExternal).toHaveBeenCalledWith(appLink);
    expect(toastError).toHaveBeenCalledWith({
      title: 'Could not open the app',
      msg: 'This app no longer exists.',
    });
  });

  it('says nothing when the app opened', async () => {
    const openExternal = vi.fn(async () => ({ ok: true }));
    installElectronMock(openExternal);
    renderAppLink();
    await userEvent.click(await screen.findByRole('button', { name: /open in default browser/i }));
    expect(openExternal).toHaveBeenCalledWith(appLink);
    expect(toastError).not.toHaveBeenCalled();
  });
});
