import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  ChromeImportRequest,
  ChromeImportStatus,
  ChromeImportSummary,
  EmbeddedBrowserState,
} from '../../../browser/types';
import WebPagePreview from '../../artifacts/WebPagePreview';
import BrowserSettingsSection from '../../settings/app/BrowserSettingsSection';
import { ChromeImportDialog } from './ChromeImportDialog';
import { CHROME_IMPORT_COPY as COPY, summaryDetails, summaryLine } from './copy';
import { resetChromeImportStatusForTests } from './useChromeImportStatus';

/**
 * The Chrome import as the person sees it (BB-21 to BB-25): the dialog, the first use card in the
 * browser panel, the ⋯ menu entry and Settings > App > Browser. Main is a fake bridge here; the
 * reading and decrypting are covered in `src/browser/chromeImport/*.test.ts`, and the real Electron
 * run in `tests/e2e/biorouter-browser-import.spec.ts`.
 */

const SUMMARY: ChromeImportSummary = {
  id: 'chrome-1',
  profileName: 'Work',
  importedAt: Date.UTC(2026, 10, 19, 12),
  logins: 'imported',
  cookies: 10,
  cookieSites: 3,
  skipped: 8,
  failed: 0,
  bookmarks: 5,
  history: 91,
  settings: ['search engine', 'languages', 'default zoom'],
};

function status(extra: Partial<ChromeImportStatus> = {}): ChromeImportStatus {
  return {
    available: true,
    offerFirstUse: true,
    loginsSupported: true,
    asksForKeyAccess: true,
    imports: [],
    ...extra,
  };
}

function installImport(initial: ChromeImportStatus) {
  let current = initial;
  const bridge = {
    status: vi.fn(async () => current),
    profiles: vi.fn(async () => [
      { id: 'Default', name: 'Work', email: 'fixture.person@example.test', lastUsed: true },
      { id: 'Profile 1', name: 'Personal', email: null, lastUsed: false },
    ]),
    sites: vi.fn(async () => ({
      readable: true as const,
      sites: [
        { site: 'site-a.test', count: 6 },
        { site: 'site-b.test', count: 5 },
        { site: 'ox.ac.uk', count: 1 },
      ],
      total: 12,
    })),
    run: vi.fn(async (_request: ChromeImportRequest) => {
      current = { ...current, offerFirstUse: false, imports: [SUMMARY] };
      return { ok: true as const, summary: SUMMARY };
    }),
    remove: vi.fn(async () => {
      current = { ...current, imports: [] };
      return true;
    }),
    dismissOffer: vi.fn(async () => {
      current = { ...current, offerFirstUse: false };
      return true;
    }),
  };
  return bridge;
}

function installElectron(bridge: ReturnType<typeof installImport>, browser?: object) {
  Object.defineProperty(window, 'electron', {
    configurable: true,
    value: { browserImport: bridge, embeddedBrowser: browser, platform: 'darwin' },
  });
}

beforeEach(() => resetChromeImportStatusForTests());
afterEach(() => vi.clearAllMocks());

async function openDialog() {
  const onImported = vi.fn();
  const onOpenChange = vi.fn();
  await act(async () => {
    render(<ChromeImportDialog open onOpenChange={onOpenChange} onImported={onImported} />);
  });
  const dialog = await screen.findByTestId('chrome-import-dialog');
  await waitFor(() => expect(within(dialog).getByRole('radio', { name: /^Work/ })).toBeTruthy());
  return { dialog, onImported, onOpenChange };
}

describe('the import dialog (BB-21)', () => {
  it('lists profiles with the last used selected and imports everything by default', async () => {
    const bridge = installImport(status());
    installElectron(bridge);
    const { dialog, onImported } = await openDialog();
    expect((within(dialog).getByRole('radio', { name: /^Work/ }) as HTMLInputElement).checked).toBe(
      true
    );
    expect(within(dialog).getByText('fixture.person@example.test')).toBeTruthy();
    expect(within(dialog).getByText(COPY.notImported)).toBeTruthy();
    expect(within(dialog).getByTestId('chrome-import-mac-prompt').textContent).toBe(COPY.macPrompt);
    await waitFor(() => expect(bridge.sites).toHaveBeenCalledWith('Default'));

    await userEvent.click(within(dialog).getByTestId('chrome-import-run'));
    expect(bridge.run).toHaveBeenCalledWith({
      profileId: 'Default',
      logins: true,
      bookmarks: true,
      history: true,
      settings: true,
      sites: null,
    });
    const summary = await screen.findByTestId('chrome-import-summary');
    expect(summary.textContent).toContain('Logins for 3 sites');
    expect(summary.textContent).toContain('5 bookmarks');
    expect(summary.textContent).toContain('91 history entries');
    expect(onImported).toHaveBeenCalledWith(SUMMARY);

    // Remove imported data asks first, as Settings does (L4); Cancel takes nothing.
    await userEvent.click(screen.getByRole('button', { name: COPY.remove }));
    const ask = await screen.findByRole('dialog', { name: COPY.confirmRemoveTitle('Work') });
    expect(ask.textContent).toContain(COPY.confirmRemoveMessage);
    expect(bridge.remove).not.toHaveBeenCalled();
    await userEvent.click(within(ask).getByRole('button', { name: COPY.cancel }));
    await waitFor(() =>
      expect(screen.queryByRole('dialog', { name: COPY.confirmRemoveTitle('Work') })).toBeNull()
    );
    expect(bridge.remove).not.toHaveBeenCalled();
    expect(screen.getByTestId('chrome-import-summary')).toBeTruthy();

    await userEvent.click(screen.getByRole('button', { name: COPY.remove }));
    const again = await screen.findByRole('dialog', { name: COPY.confirmRemoveTitle('Work') });
    await userEvent.click(within(again).getByRole('button', { name: COPY.confirmRemove }));
    expect(bridge.remove).toHaveBeenCalledWith('chrome-1');
    expect(await screen.findByText(COPY.removed)).toBeTruthy();
  });

  it('imports only the sites the person keeps, and only the parts they keep', async () => {
    const bridge = installImport(status());
    installElectron(bridge);
    const { dialog } = await openDialog();
    await userEvent.click(within(dialog).getByLabelText(COPY.history));
    await userEvent.click(await within(dialog).findByRole('button', { name: COPY.sites }));
    const sites = await within(dialog).findByTestId('chrome-import-sites');
    await userEvent.click(within(sites).getByLabelText('ox.ac.uk'));
    await userEvent.click(within(dialog).getByTestId('chrome-import-run'));
    expect(bridge.run).toHaveBeenCalledWith(
      expect.objectContaining({
        history: false,
        sites: ['site-a.test', 'site-b.test'],
      })
    );
  });

  it('returns to the choices without an error when the person cancels main’s confirmation', async () => {
    const bridge = installImport(status());
    bridge.run.mockResolvedValueOnce({
      ok: false,
      error: 'The import was canceled.',
      cancelled: true,
    } as never);
    installElectron(bridge);
    const { dialog } = await openDialog();
    await userEvent.click(within(dialog).getByTestId('chrome-import-run'));
    await waitFor(() => expect(bridge.run).toHaveBeenCalled());
    expect(await within(dialog).findByTestId('chrome-import-run')).toBeTruthy();
    expect(within(dialog).queryByRole('alert')).toBeNull();
  });

  it('will not import with logins on and no site chosen', async () => {
    const bridge = installImport(status());
    installElectron(bridge);
    const { dialog } = await openDialog();
    await userEvent.click(await within(dialog).findByRole('button', { name: COPY.sites }));
    await userEvent.click(within(dialog).getByRole('button', { name: COPY.selectNone }));
    await userEvent.click(within(dialog).getByTestId('chrome-import-run'));
    expect(bridge.run).not.toHaveBeenCalled();
    expect(within(dialog).getByRole('alert').textContent).toBe(COPY.noneSelected);
  });

  it('on Windows shows logins as unavailable and never sends them', async () => {
    const bridge = installImport(status({ loginsSupported: false, asksForKeyAccess: false }));
    installElectron(bridge);
    const { dialog } = await openDialog();
    await waitFor(() =>
      expect((within(dialog).getByLabelText(COPY.logins) as HTMLInputElement).disabled).toBe(true)
    );
    expect(within(dialog).getByText(COPY.windowsLogins)).toBeTruthy();
    expect(within(dialog).queryByTestId('chrome-import-mac-prompt')).toBeNull();
    expect(bridge.sites).not.toHaveBeenCalled();
    await userEvent.click(within(dialog).getByTestId('chrome-import-run'));
    expect(bridge.run).toHaveBeenCalledWith(expect.objectContaining({ logins: false }));
  });

  it('says why logins are missing from a summary', async () => {
    const bridge = installImport(status());
    bridge.run.mockResolvedValueOnce({
      ok: true,
      summary: { ...SUMMARY, logins: 'not-allowed', cookies: 0, cookieSites: 0 },
    });
    installElectron(bridge);
    const { dialog } = await openDialog();
    await userEvent.click(within(dialog).getByTestId('chrome-import-run'));
    const summary = await screen.findByTestId('chrome-import-summary');
    expect(summary.textContent).toContain('access was not allowed');
    expect(summary.textContent).not.toContain('Logins for');
  });

  it('says how many cookies were left out or refused, and a key that was missing (L5)', async () => {
    const bridge = installImport(status());
    bridge.run.mockResolvedValueOnce({
      ok: true,
      summary: { ...SUMMARY, skipped: 8, failed: 1, loginsMissingKey: 'not-allowed' },
    });
    installElectron(bridge);
    const { dialog } = await openDialog();
    await userEvent.click(within(dialog).getByTestId('chrome-import-run'));
    const summary = await screen.findByTestId('chrome-import-summary');
    expect(summary.textContent).toContain('Logins for 3 sites');
    expect(summary.textContent).toContain(
      'Some logins were not imported because access to Chrome’s key for them was not allowed.'
    );
    expect(summary.textContent).toContain(
      '8 cookies were left out: expired, partitioned or unreadable.'
    );
    expect(summary.textContent).toContain('1 cookie could not be set in the Biorouter Browser.');
    // An import whose every cookie was left out says so, instead of "Nothing."
    expect(
      summaryDetails({
        ...SUMMARY,
        cookies: 0,
        cookieSites: 0,
        skipped: 2,
        bookmarks: null,
        history: null,
        settings: null,
      })
    ).toBe('No logins were imported. 2 cookies were left out: expired, partitioned or unreadable.');
  });
});

function stateAt(url: string): EmbeddedBrowserState {
  return {
    url,
    title: 'Page',
    managedApp: false,
    sourceRevision: '1:0',
    canGoBack: false,
    canGoForward: false,
    isLoading: false,
    error: null,
  };
}

function fakeBrowser() {
  return {
    create: vi.fn(async () => stateAt('https://example.com/')),
    onState: vi.fn(() => () => {}),
    onShortcut: vi.fn(() => () => {}),
    setBounds: vi.fn(async () => {}),
    setVisible: vi.fn(async () => {}),
    release: vi.fn(async () => {}),
    destroy: vi.fn(async () => {}),
    control: vi.fn(async () => true),
    navigate: vi.fn(async () => true),
    clearData: vi.fn(async () => true),
    bookmarks: vi.fn(async () => [
      { url: 'https://mine.test/', title: 'Mine' },
      {
        url: 'https://pubmed.ncbi.nlm.nih.gov/',
        title: 'PubMed',
        folder: 'Imported from Chrome (Work) / Bookmarks bar',
      },
    ]),
    suggest: vi.fn(async () => []),
    searchEngine: vi.fn(async () => ({
      name: 'DuckDuckGo',
      template: 'https://duckduckgo.com/?q={searchTerms}',
    })),
  };
}

async function mountPage(allowChromeImport: boolean) {
  await act(async () => {
    render(
      <WebPagePreview
        url="https://example.com/"
        viewId="tab-1"
        onOpenExternal={vi.fn()}
        allowChromeImport={allowChromeImport}
      />
    );
  });
}

describe('the first use card and the ⋯ menu (BB-21)', () => {
  it('offers the import once in a page the person opened, with the page hidden', async () => {
    const bridge = installImport(status());
    const browser = fakeBrowser();
    installElectron(bridge, browser);
    await mountPage(true);
    const offer = await screen.findByTestId('chrome-import-offer');
    expect(offer.textContent).toContain(COPY.offerTitle);
    await waitFor(() => expect(browser.setVisible).toHaveBeenLastCalledWith('tab-1', false));

    await userEvent.click(screen.getByTestId('chrome-import-offer-not-now'));
    expect(bridge.dismissOffer).toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByTestId('chrome-import-offer')).toBeNull());
    await waitFor(() => expect(browser.setVisible).toHaveBeenLastCalledWith('tab-1', true));
  });

  it('opens the dialog from the card, keeps the page hidden, and reloads it after', async () => {
    const bridge = installImport(status());
    const browser = fakeBrowser();
    installElectron(bridge, browser);
    await mountPage(true);
    await userEvent.click(await screen.findByTestId('chrome-import-offer-import'));
    const dialog = await screen.findByTestId('chrome-import-dialog');
    expect(browser.setVisible).toHaveBeenLastCalledWith('tab-1', false);
    await waitFor(() => expect(within(dialog).getByRole('radio', { name: /^Work/ })).toBeTruthy());
    await userEvent.click(within(dialog).getByTestId('chrome-import-run'));
    await screen.findByTestId('chrome-import-summary');
    expect(browser.control).toHaveBeenCalledWith('tab-1', 'reload');
  });

  it('never offers it in a tab the agent opened or an app preview', async () => {
    const bridge = installImport(status());
    installElectron(bridge, fakeBrowser());
    await mountPage(false);
    await act(async () => {});
    expect(screen.queryByTestId('chrome-import-offer')).toBeNull();
    fireEvent.pointerDown(screen.getByTestId('browser-more'), { button: 0, ctrlKey: false });
    expect(screen.queryByRole('menuitem', { name: COPY.open })).toBeNull();
  });

  it('lists Import from Chrome in the menu, and imported bookmarks in their folder', async () => {
    const bridge = installImport(status({ offerFirstUse: false }));
    const browser = fakeBrowser();
    installElectron(bridge, browser);
    await mountPage(true);
    await waitFor(() => expect(bridge.status).toHaveBeenCalled());
    fireEvent.pointerDown(screen.getByTestId('browser-more'), { button: 0, ctrlKey: false });
    expect(await screen.findByRole('menuitem', { name: COPY.open })).toBeTruthy();
    const bookmarks = await screen.findByRole('menuitem', { name: 'Bookmarks' });
    fireEvent.keyDown(bookmarks, { key: 'ArrowRight' });
    expect(
      await screen.findByRole('menuitem', { name: 'Imported from Chrome (Work)' })
    ).toBeTruthy();
    await userEvent.click(screen.getByRole('menuitem', { name: COPY.open }));
    expect(await screen.findByTestId('chrome-import-dialog')).toBeTruthy();
  });

  it('searches with the imported search engine', async () => {
    const bridge = installImport(status({ offerFirstUse: false }));
    const browser = fakeBrowser();
    installElectron(bridge, browser);
    await mountPage(true);
    await waitFor(() => expect(browser.searchEngine).toHaveBeenCalled());
    const address = screen.getByRole('combobox', { name: 'Address' });
    await userEvent.clear(address);
    await userEvent.type(address, 'myelin repair{Enter}');
    expect(browser.navigate).toHaveBeenCalledWith(
      'tab-1',
      'https://duckduckgo.com/?q=myelin%20repair'
    );
  });
});

describe('Settings > App > Browser (BB-25)', () => {
  it('shows each import with Remove imported data, and opens the import', async () => {
    const bridge = installImport(status({ offerFirstUse: false, imports: [SUMMARY] }));
    installElectron(bridge);
    await act(async () => {
      render(<BrowserSettingsSection />);
    });
    const row = await screen.findByTestId('settings-browser-import');
    expect(row.textContent).toContain(summaryLine(SUMMARY));
    expect(summaryLine(SUMMARY)).toBe('Imported from Chrome (Work) on Nov 19');
    expect(summaryDetails(SUMMARY)).toBe(
      'Logins for 3 sites, 5 bookmarks, 91 history entries, search engine, languages, default zoom. 8 cookies were left out: expired, partitioned or unreadable.'
    );
    expect(summaryDetails({ ...SUMMARY, logins: 'not-allowed', cookies: 0 })).toBe(
      '5 bookmarks, 91 history entries, search engine, languages, default zoom. Logins were not imported because access was not allowed.'
    );
    // L4: Remove asks first, in the app's confirmation, and Cancel keeps everything.
    await userEvent.click(screen.getByRole('button', { name: COPY.removeNamed('Work') }));
    const ask = await screen.findByRole('dialog', { name: COPY.confirmRemoveTitle('Work') });
    expect(ask.textContent).toContain(COPY.confirmRemoveMessage);
    expect(bridge.remove).not.toHaveBeenCalled();
    await userEvent.click(within(ask).getByRole('button', { name: COPY.cancel }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(bridge.remove).not.toHaveBeenCalled();
    expect(screen.getByTestId('settings-browser-import')).toBeTruthy();

    await userEvent.click(screen.getByRole('button', { name: COPY.removeNamed('Work') }));
    const again = await screen.findByRole('dialog', { name: COPY.confirmRemoveTitle('Work') });
    await userEvent.click(within(again).getByRole('button', { name: COPY.confirmRemove }));
    expect(bridge.remove).toHaveBeenCalledWith('chrome-1');
    await waitFor(() => expect(screen.queryByTestId('settings-browser-import')).toBeNull());
    await userEvent.click(screen.getByTestId('settings-browser-import-open'));
    expect(await screen.findByTestId('chrome-import-dialog')).toBeTruthy();
  });

  it('cannot import when Chrome is not on this computer', async () => {
    const bridge = installImport(status({ available: false, offerFirstUse: false }));
    installElectron(bridge);
    await act(async () => {
      render(<BrowserSettingsSection />);
    });
    await waitFor(() =>
      expect(
        (screen.getByTestId('settings-browser-import-open') as HTMLButtonElement).disabled
      ).toBe(true)
    );
  });

  it('is absent where there is no desktop bridge', async () => {
    Object.defineProperty(window, 'electron', {
      configurable: true,
      value: { platform: 'darwin' },
    });
    const { container } = render(<BrowserSettingsSection />);
    expect(container.textContent).toBe('');
  });
});
