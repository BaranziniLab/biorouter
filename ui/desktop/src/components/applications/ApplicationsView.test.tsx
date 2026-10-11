import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import ApplicationsView, { ApplicationItem } from './ApplicationsView';
import { appsCopy } from './copy';

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

/** Radix opens a dropdown on pointerdown, not click. */
function openRowMenu(title = 'Cohort Explorer') {
  fireEvent.pointerDown(screen.getByRole('button', { name: appsCopy.moreActionsNamed(title) }), {
    button: 0,
    ctrlKey: false,
  });
}

vi.mock('../../toasts', () => ({
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
}));

const app = {
  id: 'cohort-explorer',
  title: 'Cohort Explorer',
  description: 'Explore a paired cohort.',
  kind: 'agentic' as const,
  session_id: 'session-1',
};

describe('ApplicationItem', () => {
  const renderItem = (overrides: Partial<Parameters<typeof ApplicationItem>[0]> = {}) =>
    render(
      <ApplicationItem
        app={app}
        onLaunch={vi.fn()}
        onOpenConversation={vi.fn()}
        onExport={vi.fn()}
        onDelete={vi.fn()}
        {...overrides}
      />
    );

  /**
   * Spec 3.10: Launch and `⋯` are the row's only visible actions, revealed on
   * hover or focus by opacity alone, so they stay in the tab order.
   */
  it('keeps Launch and the menu keyboard-reachable while they are visually hidden', async () => {
    const user = userEvent.setup();
    const onLaunch = vi.fn();
    renderItem({ onLaunch });

    const launch = screen.getByRole('button', { name: appsCopy.launchNamed('Cohort Explorer') });
    expect(launch.closest('.br-row-actions')).not.toBeNull();
    expect(
      screen.getByRole('button', { name: appsCopy.moreActionsNamed('Cohort Explorer') })
    ).toBeInTheDocument();

    await user.tab();
    expect(launch).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(onLaunch).toHaveBeenCalledTimes(1);
  });

  it('holds the other verbs in the row menu, Delete last and destructive', async () => {
    const user = userEvent.setup();
    const onOpenConversation = vi.fn();
    renderItem({ onOpenConversation });

    openRowMenu();
    const items = await screen.findAllByRole('menuitem');
    expect(items.map((item) => item.textContent)).toEqual([
      appsCopy.openChat,
      appsCopy.exportItem,
      appsCopy.delete,
    ]);
    expect(items[2]).toHaveAttribute('data-variant', 'destructive');
    // The items that open a dialog carry the ellipsis (V3-21): Export… and Delete….
    expect(items[1]).toHaveTextContent('Export…');
    expect(items[2]).toHaveTextContent('Delete…');
    await user.click(items[0]);
    expect(onOpenConversation).toHaveBeenCalledTimes(1);
  });

  it('opens the same menu on a right-click of the row', async () => {
    renderItem();

    fireEvent.contextMenu(screen.getByText('Cohort Explorer'));
    expect(await screen.findByRole('menuitem', { name: appsCopy.delete })).toBeInTheDocument();
  });

  it('refuses duplicate launch and export actions while they are in flight', async () => {
    renderItem({ isLaunching: true, isExporting: true });

    // Working, not disabled: a focused button that turns `disabled` hands focus to <body> in
    // Chromium (V3-R2-1); `launch` itself refuses a second press for the same app.
    const launch = screen.getByRole('button', { name: appsCopy.launchNamed('Cohort Explorer') });
    expect(launch).toBeEnabled();
    expect(launch).toHaveClass('br-action-busy');
    expect(screen.getByTestId('app-row')).toHaveAttribute('aria-busy', 'true');
    openRowMenu();
    expect(await screen.findByRole('menuitem', { name: appsCopy.exportItem })).toHaveAttribute(
      'aria-disabled',
      'true'
    );
  });

  /** Two lines: the title, then "{model} · {kb}" in the sans face, no icons. */
  it('says the model and the knowledge base on one sans line', () => {
    renderItem({
      app: {
        ...app,
        updated_at: 1750000000,
        agent: {
          model: { provider: 'anthropic', model: 'claude-sonnet-4-5' },
          knowledge_base: 'ms-cohort',
        },
      },
    });

    const meta = screen.getByText('claude-sonnet-4-5 · ms-cohort');
    expect(meta.tagName).toBe('P');
    expect(meta.closest('.font-mono')).toBeNull();
    // The description gives way to the facts; the row has two lines.
    expect(screen.queryByText('Explore a paired cohort.')).toBeNull();
    // At rest the row shows its relative time in the meta slot, not a date line.
    expect(screen.getByTestId('app-row').querySelector('.br-row-actions-meta time')).not.toBeNull();
  });

  it('puts the declared surface in the kind badge’s help, not in a second badge', () => {
    renderItem({
      app: {
        ...app,
        theme: { pack: 'midnight' },
        surface: {
          actions: [{ name: 'focus_node' }, { name: 'reset_view' }],
          signals: [{ name: 'node_selected' }],
        },
      },
    });

    expect(screen.queryByText('midnight')).not.toBeInTheDocument();
    const kind = screen.getByText('agentic');
    expect(kind).toHaveAccessibleDescription('2 actions · 1 signal');
  });

  it('renders no surface help for a plain v1 manifest', () => {
    renderItem({ app: { ...app, theme: { pack: 'biorouter' } } });

    expect(screen.queryByText('biorouter')).toBeNull();
    expect(screen.queryByText(/\d+ actions?/)).toBeNull();
    expect(screen.queryByText(/\d+ signals?/)).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// Export dialog (Apps SDK v2, design §3.9)
// ---------------------------------------------------------------------------

const exportableApp = {
  ...app,
  created_at: 1750000000,
  updated_at: 1750000000,
  agent: {
    model: { provider: 'anthropic', model: 'claude-sonnet-4-5' },
    extensions: ['spokeagent'],
    skills: ['ggplot-visualization'],
    knowledge_base: 'ms-cohort',
  },
};

const okJson = (body: unknown) =>
  ({ ok: true, status: 200, json: async () => body, text: async () => '' }) as Response;

// ---------------------------------------------------------------------------
// The view's four states, on the settings visual vocabulary
// ---------------------------------------------------------------------------

describe('ApplicationsView', () => {
  let fetchMock: ReturnType<typeof vi.fn>;

  const renderView = () =>
    render(
      <MemoryRouter>
        <ApplicationsView />
      </MemoryRouter>
    );

  beforeEach(() => {
    vi.clearAllMocks();
    fetchMock = vi.fn(async () => okJson([exportableApp]));
    vi.stubGlobal('fetch', fetchMock);
    window.electron = {
      platform: 'darwin',
      getSecretKey: vi.fn().mockResolvedValue('test-secret'),
      openExternal: vi.fn().mockResolvedValue(undefined),
    } as unknown as typeof window.electron;
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  /**
   * The band (spec 3.10): the title with its help in an InfoTip, the filter and
   * Refresh at the trailing edge, and no primary (an app is built from a chat).
   */
  it('renders the band with the filter and Refresh, and no primary', async () => {
    const { container } = renderView();

    expect(
      await screen.findByRole('heading', { level: 1, name: 'Built apps' })
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'About Built apps' })).toHaveAccessibleDescription(
      appsCopy.info
    );

    const strip = container.querySelector('.biorouter-page-header-actions');
    expect(strip).not.toBeNull();
    expect(strip).toContainElement(screen.getByRole('button', { name: appsCopy.refresh }));
    expect(strip).toContainElement(screen.getByRole('searchbox', { name: appsCopy.filterLabel }));
    expect(strip?.querySelector('.bg-background-accent')).toBeNull();
    expect(container.querySelector('.page-transition')).toBeNull();
  });

  it('filters the rows from the band', async () => {
    const user = userEvent.setup();
    renderView();
    expect(await screen.findByText('Cohort Explorer')).toBeInTheDocument();

    await user.type(screen.getByRole('searchbox', { name: appsCopy.filterLabel }), 'nothing');
    expect(screen.queryByText('Cohort Explorer')).toBeNull();
    expect(screen.getByRole('heading', { name: appsCopy.noMatchTitle })).toBeInTheDocument();
  });

  it('shows skeleton rows while the first load is in flight, not a line of prose', async () => {
    let release: (value: Response) => void = () => {};
    fetchMock.mockImplementationOnce(
      () =>
        new Promise<Response>((resolve) => {
          release = resolve;
        })
    );

    const { container } = renderView();

    // ⚠ Wait for the call before releasing it. `load()` awaits `secretHeader()`
    // first, so `fetch` is invoked a microtask after mount — release it any
    // earlier and `release` is still the no-op initializer, the deferred
    // promise never settles, and the test times out looking like a render bug.
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());

    expect(container.querySelectorAll('[data-slot="skeleton"]').length).toBeGreaterThan(0);
    expect(screen.queryByText(/Loading apps/)).not.toBeInTheDocument();

    await act(async () => {
      release(okJson([exportableApp]));
    });

    expect(await screen.findByText('Cohort Explorer')).toBeInTheDocument();
    expect(container.querySelectorAll('[data-slot="skeleton"]')).toHaveLength(0);
  });

  /**
   * W2-HRD-1: Launch names the app to the main process and nothing else. The
   * main process mints the app's one-time launch link and hands it to the
   * browser without putting it on a command line; the renderer neither asks for
   * a link nor opens a URL itself.
   */
  it('launches an app through the main process, naming only the app', async () => {
    const user = userEvent.setup();
    const openAppInBrowser = vi.fn().mockResolvedValue(undefined);
    window.electron = { ...window.electron, openAppInBrowser } as typeof window.electron;
    renderView();

    await user.click(
      await screen.findByRole('button', { name: appsCopy.launchNamed('Cohort Explorer') })
    );

    await waitFor(() => expect(openAppInBrowser).toHaveBeenCalledWith('cohort-explorer'));
    expect(window.electron.openExternal).not.toHaveBeenCalled();
    expect(fetchMock.mock.calls.some(([url]) => String(url).includes('/launch'))).toBe(false);
  });

  it('says why a launch failed, without the IPC preamble', async () => {
    const { toastError } = await import('../../toasts');
    const user = userEvent.setup();
    const openAppInBrowser = vi
      .fn()
      .mockRejectedValue(
        new Error(
          "Error invoking remote method 'apps:open-in-browser': Error: This app no longer exists."
        )
      );
    window.electron = { ...window.electron, openAppInBrowser } as typeof window.electron;
    renderView();

    await user.click(
      await screen.findByRole('button', { name: 'Launch Cohort Explorer in browser' })
    );

    await waitFor(() =>
      expect(toastError).toHaveBeenCalledWith({
        title: 'Cohort Explorer',
        msg: 'This app no longer exists.',
      })
    );
  });

  it('shows the shared empty state when nothing has been built', async () => {
    fetchMock.mockResolvedValue(okJson([]));

    renderView();

    const title = await screen.findByRole('heading', { level: 2, name: appsCopy.emptyTitle });
    expect(title.closest('section')).toHaveAccessibleDescription(appsCopy.emptyDescription);
    expect(within(title.closest('section') as HTMLElement).queryByRole('button')).toBeNull();
  });

  /**
   * ⚠ The rows have to SURVIVE the failure. The branch this replaces was
   * `error && apps.length === 0`, so a refresh that failed with rows on screen
   * reported nothing at all and the list silently went stale; the fix is only
   * half done if the note appears and the rows go with it.
   */
  it('reports a failed refresh in a danger note without hiding the rows it already has', async () => {
    const user = userEvent.setup();
    renderView();
    expect(await screen.findByText('Cohort Explorer')).toBeInTheDocument();

    fetchMock.mockResolvedValueOnce({
      ok: false,
      status: 500,
      text: async () => 'boom',
    } as Response);
    await user.click(screen.getByRole('button', { name: appsCopy.refresh }));

    const note = await screen.findByRole('alert');
    expect(note).toHaveTextContent(appsCopy.loadFailed('HTTP 500: boom'));
    expect(within(note).getByRole('button', { name: 'Retry' })).toBeInTheDocument();
    expect(screen.getByText('Cohort Explorer')).toBeInTheDocument();

    await user.click(within(note).getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument());
  });
});

describe('ApplicationsView export dialog', () => {
  let fetchMock: ReturnType<typeof vi.fn>;
  let directoryChooser: ReturnType<typeof vi.fn>;
  let writeFile: ReturnType<typeof vi.fn>;

  const exportCalls = () =>
    fetchMock.mock.calls.map(([url]) => String(url)).filter((url) => url.includes('/export'));

  beforeEach(() => {
    vi.clearAllMocks();
    fetchMock = vi.fn(async (url: string) => {
      if (String(url).includes('/export')) {
        return okJson({ id: exportableApp.id, files: { 'run.sh': '#!/bin/sh' } });
      }
      return okJson([exportableApp]);
    });
    vi.stubGlobal('fetch', fetchMock);
    directoryChooser = vi.fn().mockResolvedValue({ canceled: false, filePaths: ['/tmp/dest'] });
    writeFile = vi.fn().mockResolvedValue(true);
    window.electron = {
      platform: 'darwin',
      getSecretKey: vi.fn().mockResolvedValue('test-secret'),
      openExternal: vi.fn().mockResolvedValue(undefined),
      directoryChooser,
      writeFile,
    } as unknown as typeof window.electron;
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  async function openExportDialog() {
    const user = userEvent.setup();
    render(
      <MemoryRouter>
        <ApplicationsView />
      </MemoryRouter>
    );
    await screen.findByText('Cohort Explorer');
    openRowMenu();
    await user.click(await screen.findByRole('menuitem', { name: appsCopy.exportItem }));
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    return user;
  }

  it('opens a dialog on Export with launcher mode selected and the credentials note', async () => {
    await openExportDialog();

    expect(screen.getByRole('radio', { name: /^Launcher/ })).toBeChecked();
    expect(screen.getByRole('radio', { name: /^Full/ })).not.toBeChecked();
    // The safety fact stays visible, not behind help.
    expect(screen.getByText(appsCopy.exportDialog.credentials)).toBeVisible();
    expect(
      screen.getByRole('heading', { name: appsCopy.exportDialog.title('Cohort Explorer') })
    ).toBeInTheDocument();
    // No request happens just from opening the dialog.
    expect(exportCalls()).toHaveLength(0);
  });

  it('launcher mode requests mode=launcher with no include and keeps the download behavior', async () => {
    const user = await openExportDialog();

    await user.click(screen.getByRole('button', { name: appsCopy.exportDialog.export }));

    await waitFor(() => expect(exportCalls()).toHaveLength(1));
    const url = new URL(exportCalls()[0]);
    expect(url.pathname).toBe('/apps/cohort-explorer/export');
    expect(url.searchParams.get('mode')).toBe('launcher');
    expect(url.searchParams.get('bundle_daemon')).toBe('none');
    expect(url.searchParams.has('include')).toBe(false);

    // Today's save flow is preserved: pick a directory, write the scaffold.
    await waitFor(() => expect(writeFile).toHaveBeenCalled());
    expect(directoryChooser).toHaveBeenCalledTimes(1);
    expect(writeFile).toHaveBeenCalledWith('/tmp/dest/cohort-explorer/run.sh', '#!/bin/sh');
  });

  it('full mode pre-checks manifest grants and carries mode=full plus the include JSON', async () => {
    const user = await openExportDialog();

    await user.click(screen.getByRole('radio', { name: /^Full/ }));

    // Pre-checked from the app's agent config: checkbox plus name, grouped.
    const groups = {
      kb: screen.getByRole('group', { name: appsCopy.exportDialog.knowledgeBase }),
      skills: screen.getByRole('group', { name: appsCopy.exportDialog.skills }),
      extensions: screen.getByRole('group', { name: appsCopy.exportDialog.extensions }),
    };
    expect(within(groups.kb).getByRole('checkbox', { name: 'ms-cohort' })).toBeChecked();
    expect(
      within(groups.skills).getByRole('checkbox', { name: 'ggplot-visualization' })
    ).toBeChecked();
    expect(within(groups.extensions).getByRole('checkbox', { name: 'spokeagent' })).toBeChecked();
    // One help per group, not one sentence per item.
    expect(
      within(groups.kb).getByRole('button', {
        name: `About ${appsCopy.exportDialog.knowledgeBase}`,
      })
    ).toHaveAccessibleDescription(appsCopy.exportDialog.knowledgeBaseInfo);
    const daemon = screen.getByRole('checkbox', { name: appsCopy.exportDialog.includeBiorouter });
    expect(daemon).not.toBeChecked();

    await user.click(daemon);
    await user.click(screen.getByRole('button', { name: appsCopy.exportDialog.export }));

    await waitFor(() => expect(exportCalls()).toHaveLength(1));
    const url = new URL(exportCalls()[0]);
    expect(url.searchParams.get('mode')).toBe('full');
    expect(url.searchParams.get('bundle_daemon')).toBe('current');
    expect(JSON.parse(url.searchParams.get('include') ?? '{}')).toEqual({
      knowledge_bases: ['ms-cohort'],
      skills: ['ggplot-visualization'],
      extensions: ['spokeagent'],
    });
  });

  it('unchecking an include item drops it from the include JSON', async () => {
    const user = await openExportDialog();

    await user.click(screen.getByRole('radio', { name: /^Full/ }));
    await user.click(screen.getByRole('checkbox', { name: 'ms-cohort' }));
    await user.click(screen.getByRole('button', { name: appsCopy.exportDialog.export }));

    await waitFor(() => expect(exportCalls()).toHaveLength(1));
    const url = new URL(exportCalls()[0]);
    expect(JSON.parse(url.searchParams.get('include') ?? '{}')).toEqual({
      skills: ['ggplot-visualization'],
      extensions: ['spokeagent'],
    });
  });

  it('cancel closes the dialog and sends nothing', async () => {
    const user = await openExportDialog();

    await user.click(screen.getByRole('button', { name: 'Cancel' }));

    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(exportCalls()).toHaveLength(0);
    expect(directoryChooser).not.toHaveBeenCalled();
  });
});
