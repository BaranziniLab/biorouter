import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import ServeUpdateDialog, { dialogMode } from './ServeUpdateDialog';
import { ServeSidebarUpdateRow, ServeUpdateSection } from './ServeUpdateEntryPoints';
import SidebarUpdateButton from '../BioRouterSidebar/SidebarUpdateButton';
import { SidebarProvider } from '../ui/sidebar';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';
import {
  serveUpdateStore,
  ServeUpdateStore,
  type ServeUpdateStatus,
  type ServeUpdateView,
} from '../../utils/serveUpdate';

/**
 * SD-19: what a `biorouter serve` page shows for an update, and what each
 * control sends. The store is real; only its fetch is scripted.
 */

const READY: ServeUpdateStatus = {
  available: true,
  current: '1.94.1',
  latest: '1.95.0',
  phase: 'ready',
  percent: 100,
  installKind: 'package',
  runningTurns: 0,
};

function json(status: number, body: unknown) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

/** A store whose fetch gives `answers` in turn, then keeps answering a status
 *  request with `steady` (a POST that runs past the script gets a plain 202). */
function storeAnswering(steady: ServeUpdateStatus, ...answers: Response[]) {
  const sent: Array<{ url: string; init?: RequestInit }> = [];
  const store = new ServeUpdateStore({
    fetch: (async (url: string | URL | Request, init?: RequestInit) => {
      sent.push({ url: String(url), init });
      const next = answers.shift();
      if (next) return next;
      return init?.method === 'POST' ? new Response(null, { status: 202 }) : json(200, steady);
    }) as unknown as typeof fetch,
    secret: async () => 'secret',
    reload: vi.fn(),
    now: () => Date.now(),
  });
  return { store, sent };
}

async function loaded(status: ServeUpdateStatus) {
  const { store, sent } = storeAnswering(status);
  await store.poll();
  return { store, sent };
}

beforeEach(() => {
  document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

describe('the update dialog', () => {
  it('offers one restart for a ready update, with the rest behind an info tip', async () => {
    const { store, sent } = await loaded(READY);
    store.openDialog();
    render(<ServeUpdateDialog store={store} />);
    expect(screen.getByText('Update ready to install')).toBeInTheDocument();
    expect(
      screen.getByText('Biorouter restarts on 1.95.0 and this page reloads by itself.')
    ).toBeInTheDocument();
    expect(screen.getByText('1.94.1')).toBeInTheDocument();
    expect(screen.queryByTestId('serve-update-running')).toBeNull();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Restart & update/ }));
    });
    const install = sent.find((r) => r.url.endsWith('/headless/update/install'));
    expect(JSON.parse(String(install?.init?.body))).toEqual({ stopRunning: false });
  });

  it('says when chats are still working, and the click then means stop them', async () => {
    const { store, sent } = await loaded({ ...READY, runningTurns: 2 });
    store.openDialog();
    render(<ServeUpdateDialog store={store} />);
    expect(screen.getByTestId('serve-update-running')).toHaveTextContent(
      '2 chats are still working and will stop.'
    );
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Restart & update/ }));
    });
    const install = sent.find((r) => r.url.endsWith('/headless/update/install'));
    expect(JSON.parse(String(install?.init?.body))).toEqual({ stopRunning: true });
  });

  it('counts background jobs a restart would stop', async () => {
    const { store, sent } = await loaded({ ...READY, runningTurns: 0, runningWork: 1 });
    store.openDialog();
    render(<ServeUpdateDialog store={store} />);
    expect(await screen.findByTestId('serve-update-running')).toHaveTextContent(
      '1 background job is still running and will stop.'
    );
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Restart & update/ }));
    });
    const install = sent.find((r) => r.url.endsWith('/headless/update/install'));
    expect(JSON.parse(String(install?.init?.body))).toEqual({ stopRunning: true });
  });

  it('asks the daemon again when the dialog opens, so the warning is current', async () => {
    const { store, sent } = await loaded(READY);
    const before = sent.length;
    store.openDialog();
    await act(async () => {});
    expect(sent.length).toBe(before + 1);
    expect(sent[before].url.endsWith('/headless/update')).toBe(true);
  });

  it('gives an administrator one line with the exact command behind Copy', async () => {
    const command = 'sudo sh -c \'f=$(mktemp --suffix=.deb) && apt-get install -y "$f"\'';
    const { store } = await loaded({ ...READY, phase: 'needs_admin', adminCommand: command });
    store.openDialog();
    render(<ServeUpdateDialog store={store} />);
    expect(
      screen.getByText('Updating needs administrator rights on this server.')
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Copy update command/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Check again' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Restart & update/ })).toBeNull();
  });

  it('explains a copy built from source in one line', async () => {
    const { store } = await loaded({
      ...READY,
      phase: 'manual',
      installKind: 'source',
      manualReason: 'source',
    });
    store.openDialog();
    render(<ServeUpdateDialog store={store} />);
    expect(screen.getByText(/built from source/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Download from Biorouter/ })).toBeNull();
  });

  it('says a failure without red', async () => {
    const { store } = await loaded({
      ...READY,
      phase: 'error',
      error: 'apt-get failed (exit 100)',
    });
    store.openDialog();
    render(<ServeUpdateDialog store={store} />);
    expect(screen.getByText('Update failed')).toBeInTheDocument();
    const detail = screen.getByText('apt-get failed (exit 100)');
    expect(detail.className).not.toMatch(/danger/);
    expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument();
  });

  it('renders nothing off a browser page', async () => {
    delete document.documentElement.dataset.biorouterSurface;
    const { container } = render(<ServeUpdateDialog store={null} />);
    expect(container).toBeEmptyDOMElement();
  });
});

describe('dialogMode', () => {
  const view = (patch: Partial<ServeUpdateView>): ServeUpdateView => ({
    status: READY,
    reconnecting: false,
    waitedSeconds: 0,
    actionError: null,
    checkRequested: false,
    dialogOpen: true,
    result: null,
    ...patch,
  });

  it('puts a restart in progress and an unfinished update first', () => {
    expect(dialogMode(view({ reconnecting: true }))).toBe('restarting');
    expect(
      dialogMode(
        view({ result: { outcome: 'rolled_back', from: '1', to: '2', reason: null, at: 0 } })
      )
    ).toBe('result');
    expect(dialogMode(view({}))).toBe('ready');
    expect(dialogMode(view({ status: { ...READY, phase: 'up_to_date' } }))).toBeNull();
  });
});

describe('the lower-left update row on a serve page', () => {
  it('reads the browser store and opens the browser dialog', async () => {
    const { store } = await loaded(READY);
    render(
      <SidebarProvider>
        <ServeSidebarUpdateRow store={store} />
      </SidebarProvider>
    );
    const row = screen.getByTestId('sidebar-update-button');
    expect(row).toHaveAccessibleName('Update Biorouter to 1.95.0');
    expect(row).toHaveTextContent('Update to 1.95.0');
    fireEvent.click(row);
    expect(store.getSnapshot().dialogOpen).toBe(true);
  });

  it('is absent while Biorouter is up to date', async () => {
    const { store } = await loaded({ ...READY, phase: 'up_to_date', latest: '1.94.1' });
    render(
      <SidebarProvider>
        <ServeSidebarUpdateRow store={store} />
      </SidebarProvider>
    );
    expect(screen.queryByTestId('sidebar-update-button')).toBeNull();
  });

  it('is what the sidebar renders on a browser page, never the desktop updater', async () => {
    const onUpdaterEvent = vi.fn(() => () => {});
    Object.assign(window, { electron: { ...(window.electron ?? {}), onUpdaterEvent } });
    const fetchStub = vi.fn(async (_url: string | URL | Request) => json(200, READY));
    vi.stubGlobal('fetch', fetchStub);
    try {
      await act(async () => {
        render(
          <SidebarProvider>
            <SidebarUpdateButton />
          </SidebarProvider>
        );
      });
      expect(onUpdaterEvent).not.toHaveBeenCalled();
      expect(String(fetchStub.mock.calls[0]?.[0])).toMatch(/\/headless\/update$/);
      expect(await screen.findByTestId('sidebar-update-button')).toHaveTextContent(
        'Update to 1.95.0'
      );
    } finally {
      serveUpdateStore()?.stop();
      vi.unstubAllGlobals();
    }
  });
});

describe('Settings › About › Version on a serve page', () => {
  it('says what a daemon without browser updates is, instead of a dead button', async () => {
    const { store } = await loaded({ available: false, version: '1.94.1', runningTurns: 0 });
    render(<ServeUpdateSection store={store} />);
    expect(screen.getByTestId('settings-version-row')).toHaveTextContent('1.94.1');
    expect(screen.queryByRole('button', { name: 'Check for updates' })).toBeNull();
  });

  it('shows a refused check on the row', async () => {
    const { store } = storeAnswering(
      { ...READY, phase: 'up_to_date', latest: '1.94.1' },
      json(200, { ...READY, phase: 'up_to_date', latest: '1.94.1' }),
      json(403, { reason: 'not_from_the_page', message: 'Only the person using the page can.' }),
      json(200, { ...READY, phase: 'up_to_date', latest: '1.94.1' })
    );
    await store.poll();
    await store.check();
    render(<ServeUpdateSection store={store} />);
    expect(screen.getByText('Only the person using the page can.')).toBeInTheDocument();
  });

  it('shows the daemon version, not the shim placeholder', async () => {
    const { store } = await loaded({ ...READY, phase: 'up_to_date', latest: '1.94.1' });
    render(<ServeUpdateSection store={store} />);
    const row = screen.getByTestId('settings-version-row');
    expect(row).toHaveTextContent('1.94.1');
    expect(row).not.toHaveTextContent('0.0.0-browser');
    expect(screen.getByRole('button', { name: 'Check for updates' })).toBeInTheDocument();
  });

  it('turns its action into Restart to update once the update is ready', async () => {
    const { store } = await loaded(READY);
    render(<ServeUpdateSection store={store} />);
    expect(screen.getByText('Version 1.95.0 is ready')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Restart to update' }));
    expect(store.getSnapshot().dialogOpen).toBe(true);
  });
});
