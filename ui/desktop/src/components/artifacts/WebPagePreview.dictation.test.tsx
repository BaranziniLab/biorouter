import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import WebPagePreview, { type EmbeddedBrowserState } from './WebPagePreview';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';
import { DICTATION_ACTIONS, dictationMessage } from '../dictation/copy';

const privateTerms = vi.hoisted(() => ({ allowed: false }));
vi.mock('../dictation/CrewDictationButton', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../dictation/CrewDictationButton')>()),
  crewPrivateTermsAllowed: vi.fn(async () => privateTerms.allowed),
}));

/**
 * **Dictate into this app** (S15, PR-28). Owner decisions, 2026-11-10: the button is not
 * rendered while nothing in the app is focused, and it works whether or not **Dictate into
 * other apps** is on, so the preview never asks for that switch.
 */
const appUrl = 'http://127.0.0.1:51851/apps/app-a/';
const BUTTON = DICTATION_ACTIONS.dictateIntoThisApp;

function stateAt(url: string, managedApp: boolean): EmbeddedBrowserState {
  return {
    url,
    title: 'App',
    managedApp,
    sourceRevision: '1:0',
    canGoBack: false,
    canGoForward: false,
    isLoading: false,
    error: null,
  };
}

function installElectron(initial: EmbeddedBrowserState, focused: { value: boolean }) {
  let viewId = '';
  const browser = {
    create: vi.fn(async (id: string) => {
      viewId = id;
      return initial;
    }),
    onState: vi.fn(() => () => {}),
    setBounds: vi.fn(async () => {}),
    setVisible: vi.fn(async () => {}),
    destroy: vi.fn(async () => {}),
    control: vi.fn(async () => true),
    navigate: vi.fn(async () => {}),
    clearData: vi.fn(async () => true),
  };
  const electron = {
    embeddedBrowser: browser,
    dictationAnywherePreviewFocused: vi.fn(async () => focused.value),
    dictationAnywhereStartPreview: vi.fn(async () => true),
    showMessageBox: vi.fn(async () => ({ response: 0 })),
    // The switch reads as off. The preview must not consult it.
    dictationAnywhereGet: vi.fn(async () => ({
      enabled: false,
      shortcut: null,
      disclosureShown: false,
      tray: true,
      gnomeWayland: false,
    })),
  };
  Object.defineProperty(window, 'electron', { configurable: true, value: electron });
  return { electron, viewId: () => viewId };
}

async function mount(managedApp = true) {
  await act(async () => {
    render(<WebPagePreview url={appUrl} requireManagedApp={managedApp} onOpenExternal={vi.fn()} />);
  });
}

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
  privateTerms.allowed = false;
  vi.clearAllMocks();
});

describe('Dictate into this app in the live preview', () => {
  it('is not rendered while nothing in the app is focused', async () => {
    const { electron } = installElectron(stateAt(appUrl, true), { value: false });
    await mount();
    await waitFor(() => expect(electron.dictationAnywherePreviewFocused).toHaveBeenCalled());
    expect(screen.getByRole('button', { name: 'Open in browser' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: BUTTON })).not.toBeInTheDocument();
    expect(screen.queryByText(dictationMessage('MSG-APPS'))).not.toBeInTheDocument();
  });

  it('appears while a field is focused and starts with Dictate into other apps off', async () => {
    const { electron, viewId } = installElectron(stateAt(appUrl, true), { value: true });
    await mount();
    const button = await screen.findByRole('button', { name: BUTTON });
    expect(button).toBeEnabled();
    await userEvent.click(button);
    expect(electron.dictationAnywhereStartPreview).toHaveBeenCalledWith(viewId(), false);
    expect(electron.dictationAnywhereGet).not.toHaveBeenCalled();
  });

  it('offers Include my private terms from its menu and asks before using them', async () => {
    privateTerms.allowed = true;
    const { electron, viewId } = installElectron(stateAt(appUrl, true), { value: true });
    await mount();
    fireEvent.contextMenu(await screen.findByRole('button', { name: BUTTON }));
    const item = await screen.findByRole('menuitem', {
      name: DICTATION_ACTIONS.includePrivateTerms,
    });
    expect(electron.dictationAnywhereStartPreview).not.toHaveBeenCalled();
    fireEvent.click(item);
    await waitFor(() =>
      expect(electron.dictationAnywhereStartPreview).toHaveBeenCalledWith(viewId(), true)
    );
    expect(electron.showMessageBox).toHaveBeenCalledWith(
      expect.objectContaining({
        title: DICTATION_ACTIONS.includePrivateTerms,
        message: dictationMessage('MSG-PRIVATE-TERMS'),
        buttons: [DICTATION_ACTIONS.include, DICTATION_ACTIONS.publicTermsOnly],
        defaultId: 1,
      })
    );
  });

  it('starts with public terms only when the question is answered that way', async () => {
    privateTerms.allowed = true;
    const { electron, viewId } = installElectron(stateAt(appUrl, true), { value: true });
    electron.showMessageBox.mockResolvedValueOnce({ response: 1 });
    await mount();
    fireEvent.contextMenu(await screen.findByRole('button', { name: BUTTON }));
    fireEvent.click(
      await screen.findByRole('menuitem', { name: DICTATION_ACTIONS.includePrivateTerms })
    );
    await waitFor(() =>
      expect(electron.dictationAnywhereStartPreview).toHaveBeenCalledWith(viewId(), false)
    );
  });

  it('leaves the toolbar when the field loses focus and returns with it', async () => {
    const focused = { value: true };
    installElectron(stateAt(appUrl, true), focused);
    await mount();
    await screen.findByRole('button', { name: BUTTON });
    focused.value = false;
    await waitFor(
      () => expect(screen.queryByRole('button', { name: BUTTON })).not.toBeInTheDocument(),
      {
        timeout: 2_000,
      }
    );
    focused.value = true;
    expect(await screen.findByRole('button', { name: BUTTON }, { timeout: 2_000 })).toBeEnabled();
  });

  it('is not rendered in a serve browser, even with a field focused', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    const { electron } = installElectron(stateAt(appUrl, true), { value: true });
    await mount();
    await screen.findByRole('button', { name: 'Open in browser' });
    expect(screen.queryByRole('button', { name: BUTTON })).not.toBeInTheDocument();
    expect(electron.dictationAnywherePreviewFocused).not.toHaveBeenCalled();
  });

  it('is not rendered for a page that is not a Biorouter app', async () => {
    const site = 'https://example.org/start';
    const { electron } = installElectron(stateAt(site, false), { value: true });
    await act(async () => {
      render(<WebPagePreview url={site} onOpenExternal={vi.fn()} />);
    });
    await screen.findByRole('button', { name: 'Open in browser' });
    expect(screen.queryByRole('button', { name: BUTTON })).not.toBeInTheDocument();
    expect(electron.dictationAnywherePreviewFocused).not.toHaveBeenCalled();
  });
});
