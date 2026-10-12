import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ReactNode } from 'react';
import { MemoryRouter, Route, Routes, useLocation, useNavigate } from 'react-router-dom';
import ApplicationsView, { ApplicationItem } from './ApplicationsView';
import { appsCopy } from './copy';
import { notifySessionListChanged } from '../../utils/sessionListCache';

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

/** Open a row's `⋯` menu once the row is on screen, and find one of its items. */
async function rowMenuItem(title: string, item: string) {
  fireEvent.pointerDown(
    await screen.findByRole('button', { name: appsCopy.moreActionsNamed(title) }),
    { button: 0, ctrlKey: false }
  );
  return screen.findByRole('menuitem', { name: item });
}

const chatCopy = appsCopy.buildChat;

vi.mock('../../toasts', () => ({
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
  toastWarning: vi.fn(),
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

  // The click-time check can take a moment; the row says it is working, and the item does not
  // start a second check while the first is out.
  it('shows the chat check in progress on the row, and holds the item while it runs', async () => {
    const buildChat = { kind: 'available', sessionId: 'session-1', via: 'self' } as const;
    const { rerender } = renderItem({ buildChat });
    expect(screen.getByTestId('app-row')).toHaveAttribute('aria-busy', 'false');

    rerender(
      <ApplicationItem
        app={app}
        buildChat={buildChat}
        isOpeningChat
        onLaunch={vi.fn()}
        onOpenConversation={vi.fn()}
        onExport={vi.fn()}
        onDelete={vi.fn()}
      />
    );

    expect(screen.getByTestId('app-row')).toHaveAttribute('aria-busy', 'true');
    openRowMenu();
    expect(await screen.findByRole('menuitem', { name: appsCopy.openChat })).toHaveAttribute(
      'aria-disabled',
      'true'
    );
  });

  /**
   * A chat that is gone is said twice: at rest on the meta line (the menu is out of sight until
   * the row is hovered), and in the menu, where the item names the reason in place of the action
   * and cannot be chosen. Refresh checks again.
   */
  it('says a deleted build chat at rest, and names it in the menu as unavailable', async () => {
    const onOpenConversation = vi.fn();
    renderItem({ buildChat: { kind: 'deleted' }, onOpenConversation });

    expect(screen.getByTestId('app-build-chat-note')).toHaveTextContent(chatCopy.deletedNote);
    openRowMenu();
    const item = await screen.findByRole('menuitem', { name: chatCopy.deletedNote });
    expect(item).toHaveAttribute('aria-disabled', 'true');
    expect(screen.queryByRole('menuitem', { name: appsCopy.openChat })).toBeNull();

    fireEvent.click(item);
    expect(onOpenConversation).not.toHaveBeenCalled();
  });

  it('explains an app with no recorded chat instead of hiding the item', async () => {
    renderItem({ app: { ...app, session_id: undefined } });

    expect(screen.getByTestId('app-build-chat-note')).toHaveTextContent(chatCopy.noneNote);
    openRowMenu();
    expect(await screen.findByRole('menuitem', { name: chatCopy.noneNote })).toHaveAttribute(
      'aria-disabled',
      'true'
    );
  });

  it('keeps the item enabled while the check has not answered', async () => {
    renderItem({ buildChat: { kind: 'unknown' } });

    expect(screen.queryByTestId('app-build-chat-note')).toBeNull();
    openRowMenu();
    expect(await screen.findByRole('menuitem', { name: appsCopy.openChat })).not.toHaveAttribute(
      'aria-disabled'
    );
  });

  it('names a subagent run as what it is', async () => {
    renderItem({ buildChat: { kind: 'available', sessionId: 'child', via: 'subagent' } });

    openRowMenu();
    expect(
      await screen.findByRole('menuitem', { name: appsCopy.openSubagentRun })
    ).not.toHaveAttribute('aria-disabled');
  });

  /**
   * A row the daemon could not read a manifest for. Launch, the chat and Export
   * all need the manifest; Delete needs only the id and is the way out.
   */
  it('renders an unreadable app with its error and only Delete', async () => {
    const user = userEvent.setup();
    const onDelete = vi.fn();
    renderItem({
      app: {
        id: 'half-written',
        title: 'half-written',
        kind: 'static',
        broken: 'manifest.json: expected value at line 1 column 1',
      },
      onDelete,
    });

    expect(screen.getByText(appsCopy.broken.message)).toBeInTheDocument();
    expect(
      screen.getByText('manifest.json: expected value at line 1 column 1')
    ).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Launch/ })).toBeNull();
    expect(screen.queryByRole('button', { name: /More actions/ })).toBeNull();
    expect(screen.queryByRole('button', { name: /Export/ })).toBeNull();

    const remove = screen.getByRole('button', {
      name: appsCopy.broken.deleteNamed('half-written'),
    });
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

  /** A row, by its app's title. */
  const rowOf = (title: string) =>
    screen.getByText(title).closest('[data-testid="app-row"]') as HTMLElement;

  /** The row's at-rest build-chat note, or null when the chat item says enough. */
  const noteOf = (title = 'Cohort Explorer') =>
    within(rowOf(title)).queryByTestId('app-build-chat-note');

  /** Wait for the row's check to answer with `text`. */
  const waitForNote = (text: string, title = 'Cohort Explorer') =>
    waitFor(() => expect(noteOf(title)).toHaveTextContent(text));

  /** Choose "Open the chat it was built in" (or `label`) from a row's `⋯` menu. */
  const chooseChat = async (
    user: ReturnType<typeof userEvent.setup>,
    title = 'Cohort Explorer',
    label: string = appsCopy.openChat
  ) => {
    await user.click(await rowMenuItem(title, label));
  };

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

    await chooseChat(user);

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
   * The user's report: the control opened a tab that failed with "Failed to
   * resume session: Session not found", and the tab was saved, so it came back.
   * Now the row says so up front, and the menu names the reason in place of the
   * action and opens nothing.
   */
  it('says a deleted chat on the row, names it in the menu, and never navigates to it', async () => {
    const { toastError } = await import('../../toasts');
    renderView();

    await waitForNote(chatCopy.deletedNote);
    const item = await rowMenuItem('Cohort Explorer', chatCopy.deletedNote);
    expect(item).toHaveAttribute('aria-disabled', 'true');

    fireEvent.click(item);

    expect(toastError).not.toHaveBeenCalled();
    expect(screen.queryByTestId('pair')).toBeNull();
    expect(sessionReads('session-1')).toHaveLength(1);
    expect(fetchMock.mock.calls.some(([input]) => urlOfRequest(input).includes('/agent/'))).toBe(
      false
    );
  });

  // A 404 can also have been a store error (`buildChat.ts`), so a row that
  // reads "deleted" is not the last word: Refresh asks again.
  it('opens a chat that read as deleted once Refresh finds it after all', async () => {
    const user = userEvent.setup();
    renderView();
    await waitForNote(chatCopy.deletedNote);

    sessions['session-1'] = () => json(sessionRow('session-1', APP_CREATED - 60));
    await user.click(screen.getByRole('button', { name: appsCopy.refresh }));
    await waitFor(() => expect(noteOf()).toBeNull());
    await chooseChat(user);

    expect(await screen.findByTestId('pair')).toHaveTextContent('?resumeSessionId=session-1');
  });

  it('treats a chat newer than the app as no longer existing', async () => {
    sessions['session-1'] = () => json(sessionRow('session-1', APP_CREATED + 86_400));
    renderView();

    await waitForNote(chatCopy.replacedNote);
    expect(await rowMenuItem('Cohort Explorer', chatCopy.replacedNote)).toHaveAttribute(
      'aria-disabled',
      'true'
    );
  });

  // An older chat that reuses the id is caught only when it had gone quiet
  // before the app was created (`buildChat.ts` says what this cannot catch).
  it('treats a chat that went quiet before the app as no longer existing', async () => {
    sessions['session-1'] = () =>
      json(
        sessionRow('session-1', APP_CREATED - 6 * 3_600, { updated_at: iso(APP_CREATED - 3_600) })
      );
    renderView();

    await waitForNote(chatCopy.replacedNote);
  });

  it('says a chat this surface may not open cannot be opened here', async () => {
    sessions['session-1'] = () => new Response('', { status: 403 });
    renderView();

    await waitForNote(chatCopy.unreachableNote);
    expect(await rowMenuItem('Cohort Explorer', chatCopy.unreachableNote)).toHaveAttribute(
      'aria-disabled',
      'true'
    );
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

    await chooseChat(user);

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

    await chooseChat(user, 'Cohort Explorer', appsCopy.openSubagentRun);

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

    await screen.findByText('Cohort Explorer');
    await waitFor(() => expect(sessionReads('session-1')).toHaveLength(1));
    expect(noteOf()).toBeNull();

    delete sessions['session-1'];
    const item = await rowMenuItem('Cohort Explorer', appsCopy.openChat);
    expect(item).not.toHaveAttribute('aria-disabled');
    await user.click(item);

    await waitFor(() =>
      expect(toastError).toHaveBeenCalledWith({
        title: 'Cohort Explorer',
        msg: chatCopy.deletedRefusal,
      })
    );
    expect(screen.queryByTestId('pair')).toBeNull();
    await waitForNote(chatCopy.deletedNote);
  });

  it('does not navigate when the click-time check is not answered', async () => {
    const { toastError } = await import('../../toasts');
    const user = userEvent.setup();
    sessions['session-1'] = () => new Response('', { status: 503 });
    renderView();

    await chooseChat(user);

    await waitFor(() =>
      expect(toastError).toHaveBeenCalledWith({
        title: 'Cohort Explorer',
        msg: chatCopy.unansweredRefusal,
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

    await screen.findByText('Second App');
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
    await screen.findByText('Cohort Explorer');
    await waitFor(() => expect(sessionReads('session-1')).toHaveLength(1));
    expect(noteOf()).toBeNull();

    act(() => {
      notifySessionListChanged({ removed: 'session-1' });
    });

    await waitForNote(chatCopy.deletedNote);
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
    await screen.findByText('Cohort Explorer');
    await waitFor(() => expect(sessionReads('session-1')).toHaveLength(1));

    const held = holdReads();
    await chooseChat(user);
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
        msg: chatCopy.deletedRefusal,
      })
    );
    expect(screen.queryByTestId('pair')).toBeNull();
    // The stale 200 did not put the row back either.
    await waitForNote(chatCopy.deletedNote);
  });

  it('does not navigate on an answer a newer Refresh is still re-checking', async () => {
    const { toastError } = await import('../../toasts');
    const user = userEvent.setup();
    sessions['session-1'] = () => json(sessionRow('session-1', APP_CREATED - 60));
    renderView();
    await screen.findByText('Cohort Explorer');
    await waitFor(() => expect(sessionReads('session-1')).toHaveLength(1));

    const held = holdReads();
    await chooseChat(user);
    await waitFor(() => expect(held).toHaveLength(1));
    await user.click(screen.getByRole('button', { name: appsCopy.refresh }));
    await waitFor(() => expect(held).toHaveLength(2));

    await act(async () => {
      held[0](json(sessionRow('session-1', APP_CREATED - 60)));
    });
    await waitFor(() =>
      expect(toastError).toHaveBeenCalledWith({
        title: 'Cohort Explorer',
        msg: chatCopy.stillChecking,
      })
    );
    expect(screen.queryByTestId('pair')).toBeNull();

    // The Refresh's answer is the one the row keeps, and the next click opens it.
    await act(async () => {
      held[1](json(sessionRow('session-1', APP_CREATED - 60)));
    });
    sessions['session-1'] = () => json(sessionRow('session-1', APP_CREATED - 60));
    await chooseChat(user);
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
      await screen.findByText('Cohort Explorer');
      await waitFor(() => expect(sessionReads('session-1')).toHaveLength(1));

      const held = holdReads();
      await chooseChat(user);
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
    await screen.findByText('Second App');
    await waitFor(() => expect(sessionReads()).toHaveLength(2));

    const heldFirst = holdReads();
    const heldSecond: ((answer: Response) => void)[] = [];
    sessions['session-2'] = () => new Promise<Response>((resolve) => heldSecond.push(resolve));
    await chooseChat(user, 'Cohort Explorer');
    await chooseChat(user, 'Second App');
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
    await screen.findByText('Cohort Explorer');
    await waitFor(() => expect(sessionReads('session-1')).toHaveLength(1));

    delete sessions['session-1'];
    await user.click(screen.getByRole('button', { name: appsCopy.refresh }));

    await waitForNote(chatCopy.deletedNote);
    expect(sessionReads('session-1')).toHaveLength(2);
  });

  it('explains an app with no recorded chat and reads nothing for it', async () => {
    const { toastError } = await import('../../toasts');
    appsList = [{ ...builtApp, session_id: undefined }];
    renderView();

    await screen.findByText('Cohort Explorer');
    await waitForNote(chatCopy.noneNote);
    const item = await rowMenuItem('Cohort Explorer', chatCopy.noneNote);
    expect(item).toHaveAttribute('aria-disabled', 'true');
    fireEvent.click(item);

    expect(toastError).not.toHaveBeenCalled();
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
    expect(
      screen.getByRole('button', { name: appsCopy.broken.deleteNamed('half-written') })
    ).toBeEnabled();
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

  /**
   * V3-R7-2: a filter that matches nothing is the compact tile, as Workflows and Extensions draw
   * it, not the full empty state's `min-h-[clamp(17rem,48vh,25rem)]` box. The full box centred
   * its tile 84 to 92 px lower than the same state on the sibling views (Chromium measures the
   * real offset in `ApplicationsView.noMatch.browser.test.tsx`; jsdom has no layout).
   */
  it('draws a filter with no match as the compact tile, not the full empty state', async () => {
    const user = userEvent.setup();
    renderView();
    expect(await screen.findByText('Cohort Explorer')).toBeInTheDocument();

    await user.type(screen.getByRole('searchbox', { name: appsCopy.filterLabel }), 'nothing');

    const title = screen.getByRole('heading', { level: 2, name: appsCopy.noMatchTitle });
    const section = title.closest('section') as HTMLElement;
    expect(section).toHaveAccessibleDescription(appsCopy.noMatchDescription);
    expect(section.className).toContain('py-12');
    expect(section.className).not.toMatch(/min-h-/);
    // The empty state is the other state: it never shows beside a no-match.
    expect(screen.queryByRole('heading', { name: appsCopy.emptyTitle })).toBeNull();
  });

  /**
   * V3-R7-2, the other half: with nothing built there is nothing for a filter to miss, so the
   * empty state stays whatever is typed (Workflows decides it the same way), and keeps its full
   * height.
   */
  it('keeps the empty state when nothing is built, whatever the filter says', async () => {
    const user = userEvent.setup();
    fetchMock.mockResolvedValue(okJson([]));
    renderView();
    const title = await screen.findByRole('heading', { level: 2, name: appsCopy.emptyTitle });

    await user.type(screen.getByRole('searchbox', { name: appsCopy.filterLabel }), 'nothing');

    const kept = screen.getByRole('heading', { level: 2, name: appsCopy.emptyTitle });
    expect(kept).toBe(title);
    expect(kept.closest('section')).toHaveAccessibleDescription(appsCopy.emptyDescription);
    expect(kept.closest('section')?.className).toMatch(/min-h-/);
    expect(screen.queryByRole('heading', { name: appsCopy.noMatchTitle })).toBeNull();
  });

  it('brings the rows back when the filter is cleared', async () => {
    const user = userEvent.setup();
    renderView();
    expect(await screen.findByText('Cohort Explorer')).toBeInTheDocument();

    const filter = screen.getByRole('searchbox', { name: appsCopy.filterLabel });
    await user.type(filter, 'nothing');
    expect(screen.queryByText('Cohort Explorer')).toBeNull();
    await user.clear(filter);

    expect(screen.getByText('Cohort Explorer')).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: appsCopy.noMatchTitle })).toBeNull();
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

    appsRoute = () => ({ ok: false, status: 500, text: async () => 'boom' }) as Response;
    await user.click(screen.getByRole('button', { name: appsCopy.refresh }));

    const note = await screen.findByRole('alert');
    expect(note).toHaveTextContent(appsCopy.loadFailed('HTTP 500: boom'));
    expect(within(note).getByRole('button', { name: 'Retry' })).toBeInTheDocument();
    expect(screen.getByText('Cohort Explorer')).toBeInTheDocument();

    appsRoute = () => okJson([exportableApp]);
    await user.click(within(note).getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument());
  });
});

// Export: `ExportAppDialog.test.tsx` covers the dialog and the view's hand-off to the main
// process; `utils/appExport.test.ts` covers the export itself.
