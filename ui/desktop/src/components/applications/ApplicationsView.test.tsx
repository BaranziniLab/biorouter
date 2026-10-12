import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ReactNode } from 'react';
import { MemoryRouter, Route, Routes, useLocation, useNavigate } from 'react-router-dom';
import ApplicationsView, { ApplicationItem } from './ApplicationsView';
import { notifySessionListChanged } from '../../utils/sessionListCache';

vi.mock('../conversation/SearchView', () => ({
  SearchView: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

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
  it('keeps every action keyboard-accessible when hover controls are visually collapsed', async () => {
    const user = userEvent.setup();
    const onLaunch = vi.fn();

    render(
      <ApplicationItem
        app={app}
        onLaunch={onLaunch}
        onOpenConversation={vi.fn()}
        onExport={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    const launch = screen.getByRole('button', { name: 'Launch Cohort Explorer in browser' });
    expect(
      screen.getByRole('button', {
        name: 'Open the chat where Cohort Explorer was built',
      })
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Export Cohort Explorer to a folder' })
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Delete Cohort Explorer' })).toBeInTheDocument();
    expect(launch.parentElement).toHaveClass('sm:group-focus-within:opacity-100');

    await user.tab();
    expect(launch).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(onLaunch).toHaveBeenCalledTimes(1);
  });

  it('disables duplicate launch and export actions while they are in flight', () => {
    const { container } = render(
      <ApplicationItem
        app={app}
        onLaunch={vi.fn()}
        onOpenConversation={vi.fn()}
        onExport={vi.fn()}
        onDelete={vi.fn()}
        isLaunching
        isExporting
      />
    );

    expect(
      screen.getByRole('button', { name: 'Launch Cohort Explorer in browser' })
    ).toBeDisabled();
    expect(
      screen.getByRole('button', { name: 'Export Cohort Explorer to a folder' })
    ).toBeDisabled();
    expect(container.firstElementChild).toHaveAttribute('aria-busy', 'true');
  });

  // The click-time check can take a moment; a control that shows nothing
  // invites a second click or a walk to another page.
  it('shows the chat check in progress on the control itself, under the same name', () => {
    const props = {
      app,
      buildChat: { kind: 'available', sessionId: 'session-1', via: 'self' } as const,
      onLaunch: vi.fn(),
      onOpenConversation: vi.fn(),
      onExport: vi.fn(),
      onDelete: vi.fn(),
    };
    const { rerender } = render(<ApplicationItem {...props} />);
    const name = 'Open the chat where Cohort Explorer was built';
    expect(screen.getByRole('button', { name })).not.toHaveAttribute('aria-busy');
    expect(screen.getByRole('button', { name }).querySelector('.animate-spin')).toBeNull();

    rerender(<ApplicationItem {...props} isOpeningChat />);

    const chat = screen.getByRole('button', { name });
    expect(chat).toHaveAttribute('aria-busy', 'true');
    expect(chat.querySelector('svg.animate-spin')).not.toBeNull();
    expect(chat).toHaveClass('cursor-wait');
  });

  it('shows surface badges without exposing internal theme metadata', () => {
    render(
      <ApplicationItem
        app={{
          ...app,
          theme: { pack: 'midnight' },
          surface: {
            actions: [{ name: 'focus_node' }, { name: 'reset_view' }],
            signals: [{ name: 'node_selected' }],
          },
        }}
        onLaunch={vi.fn()}
        onOpenConversation={vi.fn()}
        onExport={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    expect(screen.queryByText('midnight')).not.toBeInTheDocument();
    expect(screen.getByText('2 actions · 1 signal')).toBeInTheDocument();
  });

  it('renders no surface badge for a plain v1 manifest', () => {
    render(
      <ApplicationItem
        app={{ ...app, theme: { pack: 'biorouter' } }}
        onLaunch={vi.fn()}
        onOpenConversation={vi.fn()}
        onExport={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    expect(screen.queryByText('biorouter')).toBeNull();
    expect(screen.queryByText(/\d+ actions?/)).toBeNull();
    expect(screen.queryByText(/\d+ signals?/)).toBeNull();
  });

  /**
   * ⚠ `aria-disabled`, not `disabled`. A native disabled button has
   * `pointer-events: none` (no tooltip) and leaves the tab order, so the user
   * would see a gray icon and have no way to learn the chat is gone.
   */
  it('grays out a deleted build chat but keeps it focusable, labelled and clickable', async () => {
    const user = userEvent.setup();
    const onOpenConversation = vi.fn();
    render(
      <ApplicationItem
        app={app}
        buildChat={{ kind: 'deleted' }}
        onLaunch={vi.fn()}
        onOpenConversation={onOpenConversation}
        onExport={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    const chat = screen.getByRole('button', {
      name: 'The chat where Cohort Explorer was built was deleted',
    });
    expect(chat).toHaveAttribute('aria-disabled', 'true');
    expect(chat).not.toBeDisabled();
    expect(chat).toHaveAttribute('title', 'The chat where Cohort Explorer was built was deleted');
    expect(chat).toHaveClass('opacity-50', 'cursor-not-allowed');
    // Visible without hovering the row.
    expect(screen.getByText('Build chat deleted')).toBeInTheDocument();

    await user.tab();
    await user.tab();
    expect(chat).toHaveFocus();
    // The click still reaches the view, which re-checks and says why.
    await user.keyboard('{Enter}');
    expect(onOpenConversation).toHaveBeenCalledTimes(1);
  });

  it('explains an app with no recorded chat instead of hiding the control', () => {
    render(
      <ApplicationItem
        app={{ ...app, session_id: undefined }}
        onLaunch={vi.fn()}
        onOpenConversation={vi.fn()}
        onExport={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    expect(
      screen.getByRole('button', { name: 'No chat was recorded for Cohort Explorer' })
    ).toHaveAttribute('aria-disabled', 'true');
    expect(screen.getByText('No build chat recorded')).toBeInTheDocument();
  });

  it('keeps the control enabled while the check has not answered', () => {
    render(
      <ApplicationItem
        app={app}
        buildChat={{ kind: 'unknown' }}
        onLaunch={vi.fn()}
        onOpenConversation={vi.fn()}
        onExport={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    const chat = screen.getByRole('button', {
      name: 'Open the chat where Cohort Explorer was built',
    });
    expect(chat).not.toHaveAttribute('aria-disabled');
    expect(screen.queryByText(/Build chat/)).toBeNull();
  });

  it('names a subagent run as what it is', () => {
    render(
      <ApplicationItem
        app={app}
        buildChat={{ kind: 'available', sessionId: 'child', via: 'subagent' }}
        onLaunch={vi.fn()}
        onOpenConversation={vi.fn()}
        onExport={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    expect(
      screen.getByRole('button', { name: 'Open the subagent run that built Cohort Explorer' })
    ).not.toHaveAttribute('aria-disabled');
  });

  /**
   * A row the daemon could not read a manifest for. Launch, the chat and Export
   * all need the manifest; Delete needs only the id and is the way out.
   */
  it('renders an unreadable app with its error and only Delete', async () => {
    const user = userEvent.setup();
    const onDelete = vi.fn();
    render(
      <ApplicationItem
        app={{
          id: 'half-written',
          title: 'half-written',
          kind: 'static',
          broken: 'manifest.json: expected value at line 1 column 1',
        }}
        onLaunch={vi.fn()}
        onOpenConversation={vi.fn()}
        onExport={vi.fn()}
        onDelete={onDelete}
      />
    );

    expect(screen.getByText(/can't read this app's manifest/)).toBeInTheDocument();
    expect(
      screen.getByText('manifest.json: expected value at line 1 column 1')
    ).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Launch/ })).toBeNull();
    expect(screen.queryByRole('button', { name: /chat/ })).toBeNull();
    expect(screen.queryByRole('button', { name: /Export/ })).toBeNull();

    const remove = screen.getByRole('button', { name: 'Delete half-written' });
    expect(remove).toBeEnabled();
    await user.click(remove);
    expect(onDelete).toHaveBeenCalledTimes(1);
  });
});

// ---------------------------------------------------------------------------
// "Open the chat where this app was built" (buildChat.ts)
// ---------------------------------------------------------------------------

describe('ApplicationsView build chat', () => {
  /** Unix seconds; chats are placed before or after it. */
  const APP_CREATED = 1_760_000_000;
  const iso = (secs: number) => new Date(secs * 1000).toISOString();
  const urlOfRequest = (input: unknown) => (input instanceof Request ? input.url : String(input));

  const builtApp = {
    id: 'cohort-explorer',
    title: 'Cohort Explorer',
    kind: 'agentic' as const,
    created_at: APP_CREATED,
    updated_at: APP_CREATED,
    session_id: 'session-1',
  };

  /**
   * A chat row, last written just after the app was created (or when it was
   * born, if later), as the chat that called `create_app` is: the tool's result
   * is a message, and storing it stamps `updated_at`.
   */
  const sessionRow = (id: string, createdSecs: number, extra: Record<string, unknown> = {}) => ({
    id,
    name: `Chat ${id}`,
    user_set_name: false,
    created_at: iso(createdSecs),
    updated_at: iso(Math.max(createdSecs, APP_CREATED + 5)),
    working_dir: '/tmp',
    message_count: 2,
    extension_data: {},
    ...extra,
  });

  const json = (body: unknown, status = 200) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json' },
    });

  let fetchMock: ReturnType<typeof vi.fn>;
  let appsList: unknown[];
  /** Per session id: what `GET /sessions/{id}` answers. Absent ids are 404. */
  let sessions: Record<string, () => Response | Promise<Response>>;

  const sessionReads = (id?: string) =>
    fetchMock.mock.calls
      .map(([input]) => new URL(urlOfRequest(input), 'http://localhost'))
      .filter((url) =>
        id ? url.pathname === `/sessions/${id}` : /^\/sessions\/[^/]+$/.test(url.pathname)
      );

  /** Where the router is, so a test can see whether a click navigated. */
  function PairProbe() {
    const location = useLocation();
    return (
      <div data-testid="pair">
        {location.search}|{JSON.stringify(location.state)}
      </div>
    );
  }

  const renderView = () =>
    render(
      <MemoryRouter initialEntries={['/apps']}>
        <Routes>
          <Route path="/apps" element={<ApplicationsView />} />
          <Route path="/pair" element={<PairProbe />} />
        </Routes>
      </MemoryRouter>
    );

  const chatButton = async (name: string | RegExp) => screen.findByRole('button', { name });

  beforeEach(() => {
    vi.clearAllMocks();
    appsList = [builtApp];
    sessions = {};
    fetchMock = vi.fn(async (input: unknown) => {
      const url = new URL(urlOfRequest(input), 'http://localhost');
      if (url.pathname === '/apps') return json(appsList);
      const match = url.pathname.match(/^\/sessions\/([^/]+)$/);
      if (match) {
        const answer = sessions[decodeURIComponent(match[1])];
        return answer ? answer() : new Response('', { status: 404 });
      }
      // The session-list refresh a removal broadcast triggers, and anything else.
      return new Response('', { status: 503 });
    });
    vi.stubGlobal('fetch', fetchMock);
    window.electron = {
      platform: 'darwin',
      getSecretKey: vi.fn().mockResolvedValue('test-secret'),
      getUserActionKey: vi.fn().mockResolvedValue('proof-of-person'),
      openExternal: vi.fn().mockResolvedValue(undefined),
    } as unknown as typeof window.electron;
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('sends the proof of a person on the app list as well as the secret', async () => {
    renderView();
    await screen.findByText('Cohort Explorer');

    const listCall = fetchMock.mock.calls.find(
      ([input]) => new URL(urlOfRequest(input), 'http://localhost').pathname === '/apps'
    );
    const headers = (listCall?.[1] as RequestInit | undefined)?.headers as Record<string, string>;
    expect(headers['X-Secret-Key']).toBe('test-secret');
    expect(headers['X-User-Action']).toBe('proof-of-person');
  });

  it('opens the chat that built the app, named, after checking it exists', async () => {
    const user = userEvent.setup();
    sessions['session-1'] = () =>
      json(sessionRow('session-1', APP_CREATED - 60, { name: 'Cohort app', user_set_name: true }));
    renderView();

    await user.click(await chatButton('Open the chat where Cohort Explorer was built'));

    const pair = await screen.findByTestId('pair');
    expect(pair).toHaveTextContent('?resumeSessionId=session-1');
    expect(pair).toHaveTextContent('"title":"Cohort app"');
    expect(pair).toHaveTextContent('"userSetName":true');
    // Every read carried the proof; without it the gate answers 403 for a
    // deleted chat too.
    for (const [input] of fetchMock.mock.calls) {
      if (input instanceof Request && /^\/sessions\//.test(new URL(input.url).pathname)) {
        expect(input.headers.get('X-User-Action')).toBe('proof-of-person');
      }
    }
  });

  /**
   * The user's report: the button opened a tab that failed with "Failed to
   * resume session: Session not found", and the tab was saved, so it came back.
   * Now the row says so up front and a click opens nothing.
   */
  it('grays out a deleted chat, says so, and never navigates to it', async () => {
    const { toastError } = await import('../../toasts');
    const user = userEvent.setup();
    renderView();

    const chat = await chatButton('The chat where Cohort Explorer was built was deleted');
    expect(chat).toHaveAttribute('aria-disabled', 'true');
    expect(screen.getByText('Build chat deleted')).toBeInTheDocument();

    await user.click(chat);

    await waitFor(() =>
      expect(toastError).toHaveBeenCalledWith({
        title: 'Cohort Explorer',
        msg: 'The chat where this app was built has been deleted.',
      })
    );
    expect(screen.queryByTestId('pair')).toBeNull();
    expect(fetchMock.mock.calls.some(([input]) => urlOfRequest(input).includes('/agent/'))).toBe(
      false
    );
  });

  // A 404 can also have been a store error (`buildChat.ts`), so a row that
  // reads "deleted" is not the last word: the click asks again.
  it('opens a chat that read as deleted when the click finds it after all', async () => {
    const user = userEvent.setup();
    renderView();
    const chat = await chatButton('The chat where Cohort Explorer was built was deleted');

    sessions['session-1'] = () => json(sessionRow('session-1', APP_CREATED - 60));
    await user.click(chat);

    expect(await screen.findByTestId('pair')).toHaveTextContent('?resumeSessionId=session-1');
  });

  it('treats a chat newer than the app as no longer existing', async () => {
    sessions['session-1'] = () => json(sessionRow('session-1', APP_CREATED + 86_400));
    renderView();

    expect(
      await chatButton('The chat where Cohort Explorer was built no longer exists')
    ).toHaveAttribute('aria-disabled', 'true');
    expect(screen.getByText('Build chat no longer exists')).toBeInTheDocument();
  });

  // An older chat that reuses the id is caught only when it had gone quiet
  // before the app was created (`buildChat.ts` says what this cannot catch).
  it('treats a chat that went quiet before the app as no longer existing', async () => {
    sessions['session-1'] = () =>
      json(
        sessionRow('session-1', APP_CREATED - 6 * 3_600, { updated_at: iso(APP_CREATED - 3_600) })
      );
    renderView();

    expect(
      await chatButton('The chat where Cohort Explorer was built no longer exists')
    ).toHaveAttribute('aria-disabled', 'true');
  });

  it('says a chat this surface may not open cannot be opened here', async () => {
    sessions['session-1'] = () => new Response('', { status: 403 });
    renderView();

    expect(
      await chatButton("The chat where Cohort Explorer was built can't be opened here")
    ).toHaveAttribute('aria-disabled', 'true');
  });

  it('leads to the parent conversation when a subagent run built the app', async () => {
    const user = userEvent.setup();
    sessions['session-1'] = () =>
      json(
        sessionRow('session-1', APP_CREATED - 30, {
          session_type: 'sub_agent',
          parent_session_id: 'parent-1',
        })
      );
    sessions['parent-1'] = () => json(sessionRow('parent-1', APP_CREATED - 600));
    renderView();

    await user.click(await chatButton('Open the chat where Cohort Explorer was built'));

    expect(await screen.findByTestId('pair')).toHaveTextContent('?resumeSessionId=parent-1');
  });

  it('falls back to the subagent run itself when its parent is gone', async () => {
    const user = userEvent.setup();
    sessions['session-1'] = () =>
      json(
        sessionRow('session-1', APP_CREATED - 30, {
          session_type: 'sub_agent',
          parent_session_id: 'parent-1',
        })
      );
    renderView();

    await user.click(await chatButton('Open the subagent run that built Cohort Explorer'));

    expect(await screen.findByTestId('pair')).toHaveTextContent('?resumeSessionId=session-1');
  });

  /**
   * A daemon that did not answer must not gray out the row (it may just be
   * restarting); the click re-checks, and only a real answer decides.
   */
  it('keeps an unanswered row enabled, then refuses at the click when the chat is gone', async () => {
    const { toastError } = await import('../../toasts');
    const user = userEvent.setup();
    sessions['session-1'] = () => Promise.reject(new TypeError('Failed to fetch'));
    renderView();

    const chat = await chatButton('Open the chat where Cohort Explorer was built');
    await waitFor(() => expect(sessionReads('session-1')).toHaveLength(1));
    expect(chat).not.toHaveAttribute('aria-disabled');

    delete sessions['session-1'];
    await user.click(chat);

    await waitFor(() =>
      expect(toastError).toHaveBeenCalledWith({
        title: 'Cohort Explorer',
        msg: 'The chat where this app was built has been deleted.',
      })
    );
    expect(screen.queryByTestId('pair')).toBeNull();
    expect(
      await chatButton('The chat where Cohort Explorer was built was deleted')
    ).toHaveAttribute('aria-disabled', 'true');
  });

  it('does not navigate when the click-time check is not answered', async () => {
    const { toastError } = await import('../../toasts');
    const user = userEvent.setup();
    sessions['session-1'] = () => new Response('', { status: 503 });
    renderView();

    await user.click(await chatButton('Open the chat where Cohort Explorer was built'));

    await waitFor(() =>
      expect(toastError).toHaveBeenCalledWith({
        title: 'Cohort Explorer',
        msg: "Couldn't reach Biorouter to check that chat. Try again.",
      })
    );
    expect(screen.queryByTestId('pair')).toBeNull();
  });

  it('reads a chat shared by two apps once', async () => {
    appsList = [
      builtApp,
      { ...builtApp, id: 'second-app', title: 'Second App', updated_at: APP_CREATED - 1 },
    ];
    sessions['session-1'] = () => json(sessionRow('session-1', APP_CREATED - 60));
    renderView();

    await chatButton('Open the chat where Second App was built');
    await waitFor(() => expect(sessionReads('session-1')).toHaveLength(1));
    // Give any second read the chance to show up before asserting it did not.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(sessionReads('session-1')).toHaveLength(1);
  });

  it('flips the row the moment the chat is deleted in any window, without asking again', async () => {
    sessions['session-1'] = () => json(sessionRow('session-1', APP_CREATED - 60));
    renderView();
    await waitFor(() => expect(sessionReads('session-1')).toHaveLength(1));
    expect(await chatButton('Open the chat where Cohort Explorer was built')).toBeInTheDocument();

    act(() => {
      notifySessionListChanged({ removed: 'session-1' });
    });

    expect(
      await chatButton('The chat where Cohort Explorer was built was deleted')
    ).toHaveAttribute('aria-disabled', 'true');
    expect(sessionReads('session-1')).toHaveLength(1);
  });

  /** Hold each `GET /sessions/session-1` until the test answers it, in order. */
  const holdReads = () => {
    const held: ((answer: Response) => void)[] = [];
    sessions['session-1'] = () => new Promise<Response>((resolve) => held.push(resolve));
    return held;
  };

  // ⚠ The click's own 200 is not enough once something newer has spoken: a
  // chat deleted in another window while the click was checking it must not
  // get a tab, which is exactly the failure the check exists to prevent.
  it('does not open a chat deleted while the click was checking it', async () => {
    const { toastError } = await import('../../toasts');
    const user = userEvent.setup();
    sessions['session-1'] = () => json(sessionRow('session-1', APP_CREATED - 60));
    renderView();
    const chat = await chatButton('Open the chat where Cohort Explorer was built');
    await waitFor(() => expect(sessionReads('session-1')).toHaveLength(1));

    const held = holdReads();
    await user.click(chat);
    await waitFor(() => expect(held).toHaveLength(1));
    act(() => {
      notifySessionListChanged({ removed: 'session-1' });
    });
    await act(async () => {
      held[0](json(sessionRow('session-1', APP_CREATED - 60)));
    });

    await waitFor(() =>
      expect(toastError).toHaveBeenCalledWith({
        title: 'Cohort Explorer',
        msg: 'The chat where this app was built has been deleted.',
      })
    );
    expect(screen.queryByTestId('pair')).toBeNull();
    // The stale 200 did not put the row back either.
    expect(
      await chatButton('The chat where Cohort Explorer was built was deleted')
    ).toHaveAttribute('aria-disabled', 'true');
  });

  it('does not navigate on an answer a newer Refresh is still re-checking', async () => {
    const { toastError } = await import('../../toasts');
    const user = userEvent.setup();
    sessions['session-1'] = () => json(sessionRow('session-1', APP_CREATED - 60));
    renderView();
    const chat = await chatButton('Open the chat where Cohort Explorer was built');
    await waitFor(() => expect(sessionReads('session-1')).toHaveLength(1));

    const held = holdReads();
    await user.click(chat);
    await waitFor(() => expect(held).toHaveLength(1));
    await user.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(held).toHaveLength(2));

    await act(async () => {
      held[0](json(sessionRow('session-1', APP_CREATED - 60)));
    });
    await waitFor(() =>
      expect(toastError).toHaveBeenCalledWith({
        title: 'Cohort Explorer',
        msg: 'Biorouter was still checking that chat. Try again.',
      })
    );
    expect(screen.queryByTestId('pair')).toBeNull();

    // The Refresh's answer is the one the row keeps, and the next click opens it.
    await act(async () => {
      held[1](json(sessionRow('session-1', APP_CREATED - 60)));
    });
    sessions['session-1'] = () => json(sessionRow('session-1', APP_CREATED - 60));
    await user.click(await chatButton('Open the chat where Cohort Explorer was built'));
    expect(await screen.findByTestId('pair')).toHaveTextContent('?resumeSessionId=session-1');
  });

  /** Let anything a released answer would trigger run, before asserting it did not. */
  const settle = () =>
    act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });

  /** The way off the page that the sidebar gives, outside the view. */
  function GoToSettings() {
    const navigate = useNavigate();
    return (
      <button type="button" onClick={() => navigate('/settings')}>
        Go to Settings
      </button>
    );
  }

  // ⚠ react-router's `navigate` keeps working after the view unmounts, and
  // leaving the page advances no ticket, so only the click guard stops this.
  it.each([
    ['an open chat', () => json(sessionRow('session-1', APP_CREATED - 60))],
    ['a deleted chat', () => new Response('', { status: 404 })],
  ])(
    'neither opens nor toasts when the answer for %s lands after the user left',
    async (_label, answer) => {
      const { toastError } = await import('../../toasts');
      const user = userEvent.setup();
      sessions['session-1'] = () => json(sessionRow('session-1', APP_CREATED - 60));
      render(
        <MemoryRouter initialEntries={['/apps']}>
          <GoToSettings />
          <Routes>
            <Route path="/apps" element={<ApplicationsView />} />
            <Route path="/pair" element={<PairProbe />} />
            <Route path="/settings" element={<div data-testid="settings" />} />
          </Routes>
        </MemoryRouter>
      );
      const chat = await chatButton('Open the chat where Cohort Explorer was built');
      await waitFor(() => expect(sessionReads('session-1')).toHaveLength(1));

      const held = holdReads();
      await user.click(chat);
      await waitFor(() => expect(held).toHaveLength(1));
      await user.click(screen.getByRole('button', { name: 'Go to Settings' }));
      expect(await screen.findByTestId('settings')).toBeInTheDocument();

      await act(async () => {
        held[0](answer());
      });
      await settle();

      expect(screen.getByTestId('settings')).toBeInTheDocument();
      expect(screen.queryByTestId('pair')).toBeNull();
      expect(toastError).not.toHaveBeenCalled();
    }
  );

  it('acts only on the newest click when two rows are clicked before either answers', async () => {
    const { toastError } = await import('../../toasts');
    const user = userEvent.setup();
    appsList = [
      builtApp,
      {
        ...builtApp,
        id: 'second-app',
        title: 'Second App',
        updated_at: APP_CREATED - 1,
        session_id: 'session-2',
      },
    ];
    sessions['session-1'] = () => json(sessionRow('session-1', APP_CREATED - 60));
    sessions['session-2'] = () => json(sessionRow('session-2', APP_CREATED - 60));
    renderView();
    const first = await chatButton('Open the chat where Cohort Explorer was built');
    const second = await chatButton('Open the chat where Second App was built');
    await waitFor(() => expect(sessionReads()).toHaveLength(2));

    const heldFirst = holdReads();
    const heldSecond: ((answer: Response) => void)[] = [];
    sessions['session-2'] = () => new Promise<Response>((resolve) => heldSecond.push(resolve));
    await user.click(first);
    await user.click(second);
    await waitFor(() => expect(heldSecond).toHaveLength(1));
    expect(heldFirst).toHaveLength(1);

    // The click the user moved on from answers first: no tab for it.
    await act(async () => {
      heldFirst[0](json(sessionRow('session-1', APP_CREATED - 60)));
    });
    await settle();
    expect(screen.queryByTestId('pair')).toBeNull();

    await act(async () => {
      heldSecond[0](json(sessionRow('session-2', APP_CREATED - 60)));
    });
    expect(await screen.findByTestId('pair')).toHaveTextContent('?resumeSessionId=session-2');
    expect(toastError).not.toHaveBeenCalled();
  });

  it('re-checks on Refresh', async () => {
    const user = userEvent.setup();
    sessions['session-1'] = () => json(sessionRow('session-1', APP_CREATED - 60));
    renderView();
    await waitFor(() => expect(sessionReads('session-1')).toHaveLength(1));

    delete sessions['session-1'];
    await user.click(screen.getByRole('button', { name: 'Refresh' }));

    expect(
      await chatButton('The chat where Cohort Explorer was built was deleted')
    ).toHaveAttribute('aria-disabled', 'true');
    expect(sessionReads('session-1')).toHaveLength(2);
  });

  it('explains an app with no recorded chat and reads nothing for it', async () => {
    const { toastError } = await import('../../toasts');
    const user = userEvent.setup();
    appsList = [{ ...builtApp, session_id: undefined }];
    renderView();

    const chat = await chatButton('No chat was recorded for Cohort Explorer');
    expect(chat).toHaveAttribute('aria-disabled', 'true');
    await user.click(chat);

    await waitFor(() =>
      expect(toastError).toHaveBeenCalledWith({
        title: 'Cohort Explorer',
        msg: 'No chat was recorded for this app.',
      })
    );
    expect(sessionReads()).toHaveLength(0);
    expect(screen.queryByTestId('pair')).toBeNull();
  });

  it('lists an app whose manifest cannot be read, with Delete only', async () => {
    appsList = [
      builtApp,
      { id: 'half-written', title: 'half-written', kind: 'static', broken: 'bad json' },
    ];
    sessions['session-1'] = () => json(sessionRow('session-1', APP_CREATED - 60));
    renderView();

    expect(await screen.findByText('bad json')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Delete half-written' })).toBeEnabled();
    expect(screen.queryByRole('button', { name: 'Launch half-written in browser' })).toBeNull();
    // Nothing is looked up for it.
    await waitFor(() => expect(sessionReads('session-1')).toHaveLength(1));
    expect(sessionReads()).toHaveLength(1);
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

/** The URL a fetch was made to, whether it was given a string or a `Request`. */
const urlOf = (input: unknown) => (input instanceof Request ? input.url : String(input));

describe('ApplicationsView', () => {
  let fetchMock: ReturnType<typeof vi.fn>;
  /** What `GET /apps` answers; a test swaps it to fail a refresh. */
  let appsRoute: () => Response;

  const renderView = () =>
    render(
      <MemoryRouter>
        <ApplicationsView />
      </MemoryRouter>
    );

  beforeEach(() => {
    vi.clearAllMocks();
    appsRoute = () => okJson([exportableApp]);
    // ⚠ Routed by URL. The view also checks each app's build chat, so a
    // catch-all answer (or a `mockResolvedValueOnce`) would be consumed by
    // whichever request happened to go first.
    fetchMock = vi.fn(async (input: unknown) => {
      if (new URL(urlOf(input), 'http://localhost').pathname === '/apps') return appsRoute();
      return new Response('', { status: 503 });
    });
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
   * The header comes from `Layout/PageHeader`, so this asserts the two things
   * that were the view's own before it did: that the action lands in the
   * control strip under the description rather than beside the title, and that
   * the `page-transition` class — which matches no CSS rule in this repo — did
   * not survive the move.
   */
  it('renders the shared page header with Refresh in its control strip', async () => {
    const { container } = renderView();

    expect(
      await screen.findByRole('heading', { level: 1, name: 'Built apps' })
    ).toBeInTheDocument();
    expect(screen.getByText(/Apps you built with Agent Drafter/)).toBeInTheDocument();

    const strip = container.querySelector('.biorouter-settings-control-strip');
    expect(strip).not.toBeNull();
    expect(strip).toContainElement(screen.getByRole('button', { name: 'Refresh' }));
    expect(container.querySelector('.page-transition')).toBeNull();
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
      await screen.findByRole('button', { name: 'Launch Cohort Explorer in browser' })
    );

    await waitFor(() => expect(openAppInBrowser).toHaveBeenCalledWith('cohort-explorer'));
    expect(window.electron.openExternal).not.toHaveBeenCalled();
    expect(fetchMock.mock.calls.some(([input]) => urlOf(input).includes('/launch'))).toBe(false);
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
    appsRoute = () => okJson([]);

    renderView();

    expect(
      await screen.findByRole('heading', { level: 2, name: 'No apps built yet' })
    ).toBeInTheDocument();
    expect(screen.getByText(/use Agent Drafter to build a dashboard app/)).toBeInTheDocument();
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

    appsRoute = () => ({ ok: false, status: 500, text: async () => 'boom' }) as Response;
    await user.click(screen.getByRole('button', { name: 'Refresh' }));

    const note = await screen.findByRole('alert');
    expect(note).toHaveTextContent('Could not load apps: HTTP 500: boom');
    expect(within(note).getByRole('button', { name: 'Retry' })).toBeInTheDocument();
    expect(screen.getByText('Cohort Explorer')).toBeInTheDocument();

    appsRoute = () => okJson([exportableApp]);
    await user.click(within(note).getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument());
  });
});

// Export: `ExportAppDialog.test.tsx` covers the dialog and the view's hand-off to the main
// process; `utils/appExport.test.ts` covers the export itself.
