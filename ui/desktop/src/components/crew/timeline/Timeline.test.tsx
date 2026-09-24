import { act, fireEvent, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewMessage } from '../crewApi';
import { identityCopy } from '../identity';
import { timelineCopy } from './copy';
import { HISTORY_PAGE_SIZE } from './groupMessages';
import { Timeline } from './Timeline';
import { SKELETON_DELAY_MS } from './TimelineSkeleton';
import {
  ID,
  MACHINE_STRING,
  channel,
  installResizeObserverStub,
  makeController,
  message,
  pointerAnywhere,
  renderWithController,
  run,
  snapshotFor,
} from './timelineTestUtils';
import { AUTO_READ_DWELL_MS, AUTO_READ_MIN_INTERVAL_MS } from './useAutoMarkRead';

/**
 * The timeline against a stand-in controller (ui-redesign-spec, "The timeline"
 * and the copy deck's Timeline table). jsdom has no layout, no
 * IntersectionObserver and no container queries, so what is asserted here is
 * the accessibility tree, the controller calls and the pinned strings.
 */

const page = (count: number): CrewMessage[] =>
  Array.from({ length: count }, (_, index) =>
    message({
      id: `p-${index}`,
      body: `message ${index}`,
      at: new Date(2026, 8, 22, 9, index % 60),
    })
  );

function timelineRoot(): HTMLElement {
  const root = document.querySelector<HTMLElement>('.crew-timeline');
  if (!root) throw new Error('no timeline rendered');
  return root;
}

installResizeObserverStub();

afterEach(() => {
  vi.useRealTimers();
});

describe('the channel’s start', () => {
  it('shows the pinned intro when the start is loaded, with the creator by name', () => {
    renderWithController(<Timeline />, makeController({ messages: [message()] }));
    expect(screen.getByText('Welcome to #general')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Welcome to #general' })).toBeInTheDocument();
    const intro = screen.getByText('Welcome to #general').parentElement as HTMLElement;
    expect(intro).toHaveTextContent('Alice Chen (@alice) created this channel.');
  });

  it('offers Add people to the owner only, through the add-people dialog intent', async () => {
    const controller = makeController();
    renderWithController(<Timeline />, controller);
    await userEvent.click(screen.getByRole('button', { name: timelineCopy.introAddPeople }));
    expect(controller.openDialog).toHaveBeenCalledWith({
      kind: 'add-people',
      target: 'channel',
      targetId: ID.general,
    });

    const member = makeController({
      snapshot: snapshotFor({
        actor: { id: ID.bob, uid: 1001, username: 'bob', nickname: 'Bob Lee' },
      }),
    });
    renderWithController(<Timeline />, member);
    expect(screen.getAllByText('Welcome to #general')).toHaveLength(2);
    expect(screen.getAllByRole('button', { name: timelineCopy.introAddPeople })).toHaveLength(1);
  });

  it('never offers a second “Ask my agent” — the composer’s is the only one', () => {
    renderWithController(<Timeline />, makeController());
    expect(screen.queryByRole('button', { name: /ask my agent/i })).toBeNull();
  });

  it('shows a delayed skeleton, not the empty welcome, while messages load', () => {
    vi.useFakeTimers();
    renderWithController(<Timeline />, makeController({ messagesLoaded: false }));
    const log = screen.getByRole('log', { name: 'general messages' });
    expect(log).toHaveAttribute('aria-busy', 'true');
    expect(screen.queryByText('Welcome to #general')).toBeNull();
    expect(document.querySelector('.crew-skeleton-row')).toBeNull();
    act(() => {
      vi.advanceTimersByTime(SKELETON_DELAY_MS);
    });
    expect(document.querySelectorAll('.crew-skeleton-row').length).toBeGreaterThan(0);
    expect(within(log).getByText(timelineCopy.loadingMessages)).toBeInTheDocument();
  });

  it('renders nothing without a verified snapshot and channel', () => {
    renderWithController(<Timeline />, makeController({ snapshot: null, channel: null }));
    expect(document.querySelector('.crew-timeline')).toBeNull();
  });
});

describe('the log', () => {
  it('is a polite, focusable log named for the channel', () => {
    renderWithController(<Timeline />, makeController({ messages: [message()] }));
    const log = screen.getByRole('log', { name: 'general messages' });
    expect(log).toHaveAttribute('aria-live', 'polite');
    expect(log).toHaveAttribute('tabindex', '0');
    expect(log).not.toHaveAttribute('aria-busy');
    expect(timelineRoot().querySelector('.biorouter-scroll-fade-top')).not.toBeNull();
  });

  it('groups a person’s messages under one head with a short time and the full date in a tooltip', async () => {
    const messages = [
      message({ id: 'a', body: 'Counts are in.', at: new Date(2026, 8, 22, 10, 2) }),
      message({ id: 'b', body: 'Plot next?', at: new Date(2026, 8, 22, 10, 3) }),
    ];
    renderWithController(<Timeline />, makeController({ messages }));
    const articles = screen.getAllByRole('article');
    expect(articles).toHaveLength(1);
    expect(articles[0]).toHaveAccessibleName(/Bob Lee.*10:02 AM/);
    expect(within(articles[0]).getByText('Counts are in.')).toBeInTheDocument();
    expect(within(articles[0]).getByText('Plot next?')).toBeInTheDocument();
    const time = within(articles[0]).getAllByText('10:02 AM')[0];
    expect(time.tagName).toBe('TIME');
    await userEvent.hover(time);
    expect(await screen.findByRole('tooltip')).toHaveTextContent(
      'Tuesday, September 22, 2026 at 10:02 AM'
    );
  });

  it('reads an agent’s post as the agent, never as its owner', () => {
    const messages = [
      message({ id: 'h', actor_id: ID.bob, body: 'Can you sum the columns?' }),
      message({ id: 'mine', actor_id: ID.alice, run_id: ID.run, body: 'The totals are 1.80.' }),
      message({ id: 'theirs', actor_id: ID.bob, run_id: ID.runB, body: 'Plot attached.' }),
    ];
    renderWithController(<Timeline />, makeController({ messages }));
    const [human, mine, theirs] = screen.getAllByRole('article');
    expect(human).not.toHaveTextContent(timelineCopy.agentBadge);
    expect(mine).toHaveTextContent(`${identityCopy.yourAgent} @alice${timelineCopy.agentBadge}`);
    expect(theirs).toHaveTextContent(`Bob Lee's agent @bob${timelineCopy.agentBadge}`);
    // The agent tile is square with the Bot glyph; a person's is a circle with initials.
    expect(mine.querySelector('[data-slot="avatar"]')).toHaveAttribute('data-shape', 'square');
    expect(human.querySelector('[data-slot="avatar"]')).toHaveAttribute('data-shape', 'circle');
    expect(human.querySelector('[data-slot="avatar"]')).toHaveTextContent('BL');
  });

  it('names an author it does not know as “Unknown member”, never by ID', () => {
    renderWithController(
      <Timeline />,
      makeController({ messages: [message({ actor_id: ID.gone, body: 'hello' })] })
    );
    expect(screen.getByRole('article')).toHaveTextContent(identityCopy.unknownMember);
    expect(timelineRoot().innerHTML).not.toContain(ID.gone);
  });

  it('shows Restricted only where a message differs from its channel', () => {
    const messages = [
      message({ id: 'r', restricted: true, body: 'private note' }),
      message({ id: 'p', restricted: false, body: 'open note', actor_id: ID.carol }),
    ];
    const { unmount } = renderWithController(<Timeline />, makeController({ messages }));
    expect(screen.getAllByText(timelineCopy.restricted)).toHaveLength(1);
    expect(screen.getByText(timelineCopy.restricted)).toHaveTextContent(
      timelineCopy.restrictedTooltip
    );
    unmount();

    renderWithController(
      <Timeline />,
      makeController({ messages, channel: { ...channel, classification: 'restricted' } })
    );
    expect(screen.queryByText(timelineCopy.restricted)).toBeNull();
  });

  it('divides days and draws the New line in accent, before the first unread message', () => {
    const messages = [
      message({ id: 'a', sequence: 's1', at: new Date(2025, 5, 3, 9, 0) }),
      message({ id: 'b', sequence: 's2', at: new Date(2026, 8, 22, 9, 0), body: 'fresh' }),
    ];
    renderWithController(
      <Timeline />,
      makeController({
        messages,
        snapshot: snapshotFor({
          read_positions: { [ID.general]: 's1' },
          unread: { [ID.general]: 1 },
        }),
      })
    );
    expect(screen.getByRole('separator', { name: 'June 3, 2025' })).toBeInTheDocument();
    const newLine = screen.getByRole('separator', { name: timelineCopy.newLineLabel });
    expect(within(newLine).getByText(timelineCopy.newLine)).toHaveClass('text-text-accent');
    expect(newLine.outerHTML).not.toMatch(/danger/);
    // The line sits right before the unread message.
    const fresh = screen.getByText('fresh');
    expect(newLine.compareDocumentPosition(fresh) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('relabels Today as Yesterday at midnight, with nothing new arriving', () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    vi.setSystemTime(new Date(2026, 8, 22, 23, 59, 30));
    renderWithController(
      <Timeline />,
      makeController({ messages: [message({ at: new Date(2026, 8, 22, 9, 0) })] })
    );
    expect(screen.getByRole('separator', { name: 'Today' })).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(screen.getByRole('separator', { name: 'Yesterday' })).toBeInTheDocument();
  });

  it('keeps the New line where it was when the channel opened', () => {
    const messages = [message({ id: 'a', sequence: 's1' }), message({ id: 'b', sequence: 's2' })];
    const opened = makeController({
      messages,
      snapshot: snapshotFor({ read_positions: { [ID.general]: 's1' } }),
    });
    const { rerenderWith } = renderWithController(<Timeline />, opened);
    expect(screen.getByRole('separator', { name: timelineCopy.newLineLabel })).toBeInTheDocument();
    // The channel was marked read; a new state frame moves the position.
    rerenderWith({
      ...opened,
      snapshot: snapshotFor({ read_positions: { [ID.general]: 's2' } }),
    });
    expect(screen.getByRole('separator', { name: timelineCopy.newLineLabel })).toBeInTheDocument();
  });

  it('folds an agent’s tool updates behind “Show details”', async () => {
    const messages = [
      message({ id: 't', actor_id: ID.alice, run_id: ID.run, body: 'Task: Sum the columns' }),
      message({ actor_id: ID.alice, run_id: ID.run, body: 'Using crew__request' }),
      message({
        actor_id: ID.alice,
        run_id: ID.run,
        body: 'Tool failed: blob.read. Inspect the task conversation for details.',
      }),
      message({ actor_id: ID.alice, run_id: ID.run, body: 'The totals are 1.80.' }),
    ];
    renderWithController(<Timeline />, makeController({ messages }));
    expect(screen.queryByText(/Tool failed/)).toBeNull();
    expect(screen.getByText('The totals are 1.80.')).toBeInTheDocument();
    const details = screen.getByRole('button', { name: timelineCopy.showDetails });
    expect(details).toHaveAccessibleDescription(timelineCopy.detailsSummary(2));
    await userEvent.click(details);
    expect(screen.getByText('Using crew__request')).toBeInTheDocument();
    expect(screen.getByText(/Tool failed: blob\.read/)).toBeInTheDocument();
  });

  it('passes each message to the attachments slot', () => {
    const renderAttachments = vi.fn((item: CrewMessage) => <span>{`files for ${item.body}`}</span>);
    renderWithController(
      <Timeline renderAttachments={renderAttachments} />,
      makeController({ messages: [message({ body: 'see file', attachments: ['blob-1'] })] })
    );
    expect(screen.getByText('files for see file')).toBeInTheDocument();
  });

  it('renders no machine ID anywhere in its DOM', () => {
    const uuid = (n: number) => `0000000${n}-0000-4000-8000-00000000000${n}`;
    const messages = [
      message({ id: uuid(1), sequence: uuid(2), actor_id: ID.bob, body: 'hello' }),
      message({ id: uuid(3), actor_id: ID.gone, body: 'from a stranger' }),
      message({ id: uuid(4), actor_id: ID.alice, run_id: ID.run, body: 'Task: Plot it' }),
      message({ id: uuid(5), actor_id: ID.alice, run_id: ID.run, body: 'Using crew__request' }),
    ];
    renderWithController(
      <Timeline />,
      makeController({
        messages,
        runs: [run({ status: 'failed', error: 'Model refused' })],
        snapshot: snapshotFor({ read_positions: { [ID.general]: uuid(2) } }),
      })
    );
    const html = timelineRoot().innerHTML;
    expect(html).not.toMatch(MACHINE_STRING);
  });
});

describe('copying', () => {
  /** user-event installs its own clipboard on setup, so spy on the one it installed. */
  function setupWithClipboard() {
    const user = userEvent.setup(pointerAnywhere);
    const writeText = vi.spyOn(navigator.clipboard, 'writeText');
    return { user, writeText };
  }

  it('copies the text and, from ⋯, the message ID, and says so without a toast', async () => {
    renderWithController(
      <Timeline />,
      makeController({ messages: [message({ id: 'msg-7', body: 'Counts are **in**.' })] })
    );
    const { user, writeText } = setupWithClipboard();
    await user.click(screen.getByRole('button', { name: timelineCopy.copyText }));
    expect(writeText).toHaveBeenCalledWith('Counts are **in**.');
    expect(await screen.findByRole('status')).toHaveTextContent(timelineCopy.copied);

    await user.click(screen.getByRole('button', { name: timelineCopy.moreActions }));
    await user.click(await screen.findByRole('menuitem', { name: timelineCopy.copyMessageId }));
    expect(writeText).toHaveBeenLastCalledWith('msg-7');
    expect(document.querySelector('.Toastify')).toBeNull();
  });

  it('says how to copy by hand when the clipboard refuses', async () => {
    renderWithController(<Timeline />, makeController({ messages: [message()] }));
    const { user, writeText } = setupWithClipboard();
    writeText.mockRejectedValueOnce(new Error('denied'));
    await user.click(screen.getByRole('button', { name: timelineCopy.copyText }));
    expect(await screen.findByRole('status')).toHaveTextContent(timelineCopy.copyFailed);
  });
});

describe('older history', () => {
  it('offers “Older messages” on a full page and loads it without an IntersectionObserver', () => {
    expect(typeof IntersectionObserver).toBe('undefined');
    const controller = makeController({ messages: page(HISTORY_PAGE_SIZE) });
    renderWithController(<Timeline />, controller);
    expect(screen.queryByText('Welcome to #general')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Older messages' }));
    expect(controller.loadOlder).toHaveBeenCalledTimes(1);
  });

  it('has no sentinel once the start is loaded', () => {
    renderWithController(<Timeline />, makeController({ messages: page(3) }));
    expect(screen.queryByRole('button', { name: 'Older messages' })).toBeNull();
  });

  it('loads by itself only after the reader scrolls up', () => {
    type ObserverCallback = (
      entries: IntersectionObserverEntry[],
      observer: IntersectionObserver
    ) => void;
    const observers: { callback: ObserverCallback }[] = [];
    class FakeObserver {
      constructor(callback: ObserverCallback) {
        observers.push({ callback });
      }
      observe() {}
      disconnect() {}
      unobserve() {}
      takeRecords() {
        return [];
      }
    }
    vi.stubGlobal('IntersectionObserver', FakeObserver);
    try {
      const controller = makeController({ messages: page(HISTORY_PAGE_SIZE) });
      renderWithController(<Timeline />, controller);
      const reach = () =>
        act(() => {
          observers[observers.length - 1]?.callback(
            [{ isIntersecting: true } as IntersectionObserverEntry],
            {} as IntersectionObserver
          );
        });
      // In view the moment the page lands: not a request for more.
      reach();
      expect(controller.loadOlder).not.toHaveBeenCalled();

      const viewport = document.querySelector<HTMLElement>('[data-radix-scroll-area-viewport]');
      if (!viewport) throw new Error('no viewport');
      viewport.scrollTop = 400;
      fireEvent.scroll(viewport);
      viewport.scrollTop = 0;
      fireEvent.scroll(viewport);
      reach();
      expect(controller.loadOlder).toHaveBeenCalledTimes(1);
      // Once per arming.
      reach();
      expect(controller.loadOlder).toHaveBeenCalledTimes(1);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('loads older pages in the controller’s order without a single row rising in or a jump to the bottom', () => {
    // useCrewController.loadOlder only moves the boundary to the first message on
    // screen; useCrewObservation clears the list a render later and then puts the
    // page in. For that first render the previous page is still drawn.
    const pageOf = (label: string) =>
      Array.from({ length: HISTORY_PAGE_SIZE }, (_, index) =>
        message({
          id: `${label}-${index}`,
          body: `${label} ${index}`,
          at: new Date(2026, 8, 22, 9, index % 60),
        })
      );
    const oldest = pageOf('oldest');
    const older = pageOf('older');
    const live = pageOf('live');
    const arrivingRows = () => document.querySelectorAll('[data-arriving="true"]').length;

    const controller = makeController({ messages: live });
    const { rerenderWith } = renderWithController(<Timeline />, controller);
    const viewport = document.querySelector<HTMLElement>('[data-radix-scroll-area-viewport]');
    if (!viewport) throw new Error('no viewport');
    const scrollTo = vi.fn();
    viewport.scrollTo = scrollTo as typeof viewport.scrollTo;
    const log = screen.getByRole('log');

    // 1. The boundary moves; the live tail is still the list.
    rerenderWith({ ...controller, messages: live, historyBefore: live[0].sequence });
    expect(scrollTo).not.toHaveBeenCalled();
    expect(log).toHaveAttribute('aria-busy', 'true');
    expect(screen.getByRole('button', { name: timelineCopy.loadingOlder })).toBeDisabled();
    // 2. The list is cleared while the page is fetched.
    rerenderWith({
      ...controller,
      messages: [],
      messagesLoaded: false,
      historyBefore: live[0].sequence,
    });
    expect(scrollTo).not.toHaveBeenCalled();
    // 3. The page lands: opened at its newest message, and nothing on it is an arrival.
    rerenderWith({ ...controller, messages: older, historyBefore: live[0].sequence });
    expect(screen.getByText('older 199')).toBeInTheDocument();
    expect(scrollTo).toHaveBeenCalledTimes(1);
    expect(scrollTo).toHaveBeenLastCalledWith(expect.objectContaining({ behavior: 'auto' }));
    expect(arrivingRows()).toBe(0);
    expect(log).not.toHaveAttribute('aria-busy');

    // The next page, with the previous list handed over as a copy this time.
    rerenderWith({ ...controller, messages: [...older], historyBefore: older[0].sequence });
    expect(scrollTo).toHaveBeenCalledTimes(1);
    expect(arrivingRows()).toBe(0);
    rerenderWith({
      ...controller,
      messages: [],
      messagesLoaded: false,
      historyBefore: older[0].sequence,
    });
    rerenderWith({ ...controller, messages: oldest, historyBefore: older[0].sequence });
    expect(screen.getByText('oldest 199')).toBeInTheDocument();
    expect(scrollTo).toHaveBeenCalledTimes(2);
    expect(arrivingRows()).toBe(0);

    // Jump to latest clears the page and refreshes: the live tail opens, not arrives…
    rerenderWith({ ...controller, messages: [], messagesLoaded: false, historyBefore: null });
    rerenderWith({ ...controller, messages: live, historyBefore: null });
    expect(scrollTo).toHaveBeenCalledTimes(3);
    expect(arrivingRows()).toBe(0);
    // …and a post after it is a live arrival again.
    rerenderWith({
      ...controller,
      messages: [...live, message({ id: 'after', body: 'just posted' })],
      historyBefore: null,
    });
    expect(screen.getByText('just posted').closest('[data-crew-row]')).toHaveAttribute(
      'data-arriving',
      'true'
    );
    expect(arrivingRows()).toBe(1);
  });

  it('marks the log busy while a page loads', () => {
    renderWithController(
      <Timeline />,
      makeController({ messages: [], messagesLoaded: false, historyBefore: 's100' })
    );
    expect(screen.getByRole('log')).toHaveAttribute('aria-busy', 'true');
    expect(screen.getByText(timelineCopy.loadingOlder)).toBeInTheDocument();
  });

  it('shows the pinned history pill with Jump to latest while a page is shown', async () => {
    const controller = makeController({ messages: page(5), historyBefore: 's100' });
    renderWithController(<Timeline />, controller);
    expect(screen.getByText('Viewing earlier messages')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: timelineCopy.jumpToLatest }));
    expect(controller.jumpToLatest).toHaveBeenCalledTimes(1);
  });

  it('shows no pill on the live tail at the bottom', () => {
    renderWithController(<Timeline />, makeController({ messages: page(5) }));
    expect(screen.queryByText('Viewing earlier messages')).toBeNull();
    expect(screen.queryByRole('button', { name: timelineCopy.jumpToLatest })).toBeNull();
  });
});

describe('arrivals', () => {
  it('lets a message that arrives while following rise in, and nothing that was already there', () => {
    const first = [message({ id: 'old', body: 'old' })];
    const controller = makeController({ messages: first });
    const { rerenderWith } = renderWithController(<Timeline />, controller);
    const oldRow = screen.getByText('old').closest('[data-crew-row]');
    expect(oldRow).not.toHaveAttribute('data-arriving');

    rerenderWith({ ...controller, messages: [...first, message({ id: 'new', body: 'new' })] });
    expect(screen.getByText('new').closest('[data-crew-row]')).toHaveAttribute(
      'data-arriving',
      'true'
    );
    expect(screen.getByText('old').closest('[data-crew-row]')).not.toHaveAttribute('data-arriving');
  });
});

describe('automatic mark-read', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  const unread = (extra = {}) =>
    makeController({
      messages: [message({ id: 'a', sequence: 's1' }), message({ id: 'b', sequence: 's2' })],
      snapshot: snapshotFor({
        read_positions: { [ID.general]: 's1' },
        unread: { [ID.general]: 1 },
      }),
      ...extra,
    });

  it('marks the channel read to its newest message after a second at the bottom, without a refresh', () => {
    const controller = unread();
    renderWithController(<Timeline />, controller);
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS - 1);
    });
    expect(controller.markRead).not.toHaveBeenCalled();
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(controller.markRead).toHaveBeenCalledWith(ID.general, 's2');
    expect(controller.refresh).not.toHaveBeenCalled();
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_MIN_INTERVAL_MS * 2);
    });
    expect(controller.markRead).toHaveBeenCalledTimes(1);
  });

  it('waits five seconds before marking the same channel again', () => {
    const controller = unread();
    const { rerenderWith } = renderWithController(<Timeline />, controller);
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS);
    });
    expect(controller.markRead).toHaveBeenCalledTimes(1);
    rerenderWith({
      ...controller,
      messages: [...controller.messages, message({ id: 'c', sequence: 's3' })],
    });
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS);
    });
    expect(controller.markRead).toHaveBeenCalledTimes(1);
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_MIN_INTERVAL_MS);
    });
    expect(controller.markRead).toHaveBeenCalledTimes(2);
    expect(controller.markRead).toHaveBeenLastCalledWith(ID.general, 's3');
  });

  it('does nothing for a read channel, a history page, a read-only view or an unfocused window', () => {
    const read = unread({
      snapshot: snapshotFor({
        read_positions: { [ID.general]: 's2' },
        unread: { [ID.general]: 0 },
      }),
    });
    renderWithController(<Timeline />, read);
    const history = unread({ historyBefore: 's9' });
    renderWithController(<Timeline />, history);
    const readOnly = unread();
    renderWithController(<Timeline readOnly />, readOnly);
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_MIN_INTERVAL_MS);
    });
    expect(read.markRead).not.toHaveBeenCalled();
    expect(history.markRead).not.toHaveBeenCalled();
    expect(readOnly.markRead).not.toHaveBeenCalled();

    vi.mocked(document.hasFocus).mockReturnValue(false);
    const unfocused = unread();
    renderWithController(<Timeline />, unfocused);
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_MIN_INTERVAL_MS);
    });
    expect(unfocused.markRead).not.toHaveBeenCalled();
    // Focus returns: the gate re-runs.
    vi.mocked(document.hasFocus).mockReturnValue(true);
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS);
    });
    expect(unfocused.markRead).toHaveBeenCalledWith(ID.general, 's2');
  });

  it('stays silent when the write fails', async () => {
    const controller = unread({
      markRead: vi.fn(async () => Promise.reject(new Error('offline'))),
    });
    renderWithController(<Timeline />, controller);
    await act(async () => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS);
    });
    expect(controller.markRead).toHaveBeenCalledTimes(1);
    expect(controller.reportError).not.toHaveBeenCalled();
  });
});

describe('keyboard', () => {
  it('moves between rows with the arrow keys, putting only that row’s actions in the Tab order', () => {
    const messages = [
      message({ id: 'a', body: 'first', actor_id: ID.bob }),
      message({ id: 'b', body: 'second', actor_id: ID.carol }),
    ];
    renderWithController(<Timeline />, makeController({ messages }));
    const log = screen.getByRole('log');
    const rows = log.querySelectorAll<HTMLElement>('[data-crew-row]');
    const copyButtons = () => screen.getAllByRole('button', { name: timelineCopy.copyText });
    expect(copyButtons().every((button) => button.tabIndex === -1)).toBe(true);

    log.focus();
    fireEvent.keyDown(log, { key: 'ArrowUp' });
    expect(rows[1]).toHaveFocus();
    expect(copyButtons()[1].tabIndex).toBe(0);
    expect(copyButtons()[0].tabIndex).toBe(-1);

    fireEvent.keyDown(rows[1], { key: 'ArrowUp' });
    expect(rows[0]).toHaveFocus();
    fireEvent.keyDown(rows[0], { key: 'End' });
    expect(rows[1]).toHaveFocus();
    fireEvent.keyDown(rows[1], { key: 'Home' });
    expect(rows[0]).toHaveFocus();
  });
});

describe('a read-only view (re-verification)', () => {
  it('draws the given view dimmed, with nothing that acts', () => {
    const controller = makeController({ snapshot: null, channel: null });
    renderWithController(
      <Timeline
        readOnly
        view={{
          snapshot: snapshotFor(),
          channel,
          messages: page(HISTORY_PAGE_SIZE),
          messagesLoaded: true,
          runs: [run()],
          labels: null,
          historyBefore: null,
        }}
      />,
      controller
    );
    expect(timelineRoot()).toHaveAttribute('data-readonly', 'true');
    expect(screen.getByRole('button', { name: 'Older messages' })).toBeDisabled();
    expect(screen.getByRole('button', { name: timelineCopy.taskStopLabel })).toBeDisabled();
    expect(screen.getByRole('button', { name: timelineCopy.taskOpenLabel })).toBeDisabled();
  });
});

describe('highlighting a task', () => {
  it('scrolls the task row into view, washes it once, and reports when the wash ends', () => {
    const scrollIntoView = vi.spyOn(Element.prototype, 'scrollIntoView');
    const onHighlightDone = vi.fn();
    try {
      renderWithController(
        <Timeline highlightRunId={ID.run} onHighlightDone={onHighlightDone} />,
        makeController({
          messages: [message({ actor_id: ID.alice, run_id: ID.run, body: 'Task: Plot it' })],
          runs: [run()],
        })
      );
      const row = screen.getByRole('group', { name: /Your agent/ });
      expect(row).toHaveClass('crew-highlight');
      expect(scrollIntoView).toHaveBeenCalledWith(expect.objectContaining({ block: 'center' }));
      fireEvent.animationEnd(row);
      expect(row).not.toHaveClass('crew-highlight');
      expect(onHighlightDone).toHaveBeenCalledTimes(1);
    } finally {
      scrollIntoView.mockRestore();
    }
  });
});
