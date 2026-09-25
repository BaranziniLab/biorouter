import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewMessage } from '../crewApi';
import { identityCopy } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { timelineCopy } from './copy';
import { HISTORY_PAGE_SIZE } from './groupMessages';
import { PENDING_POST_TIMEOUT_MS, Timeline } from './Timeline';
import type { AttachmentSlotState } from './TimelineContext';
import { TimelineCopyProvider, useTimelineCopy } from './TimelineCopy';
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
import { OPENING_QUIET_MS, OPENING_STALL_MS } from './useOpening';
import { ChannelHeader } from '../channel/ChannelHeader';
import { channelHeaderCopy } from '../channel/headerCopy';
import { Composer } from '../composer/Composer';

// The composer's upload hook lists this channel's transfers; nothing here uploads.
vi.mock('../crewTransfers', () => ({
  beginTransfer: vi.fn(),
  listTransfers: vi.fn(async () => []),
  pauseTransfer: vi.fn(),
  resumeTransfer: vi.fn(),
  forgetTransfer: vi.fn(),
  previewAttachment: vi.fn(),
  clearPublishedTransfers: vi.fn(),
}));

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

/**
 * The live tail has stopped growing long enough to count as arrived. Tests hand
 * the timeline a whole list at once, which it cannot tell from the first frames
 * of a stream (the observer sends one message per frame), so a list that is
 * neither empty nor a full page opens after `OPENING_QUIET_MS`. Needs fake timers.
 */
function openFully() {
  act(() => {
    vi.advanceTimersByTime(OPENING_QUIET_MS);
  });
}

/** A message posted after the page opened: a live arrival. */
const postedNow = (overrides: Partial<CrewMessage> = {}) =>
  message({ at: new Date(Date.now() + 1000), ...overrides });

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
    vi.useFakeTimers();
    renderWithController(<Timeline />, makeController({ messages: [message()] }));
    // Until the tail has arrived its place is kept, claiming nothing.
    expect(screen.queryByRole('heading', { name: 'Welcome to #general' })).toBeNull();
    openFully();
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

  it('tells a member only whom to ask for other channels: the host (T-28, Q2-64)', () => {
    const member = makeController({
      snapshot: snapshotFor({
        actor: { id: ID.bob, uid: 1001, username: 'bob', nickname: 'Bob Lee' },
      }),
    });
    renderWithController(<Timeline />, member);
    expect(
      screen.getByText('Ask @alice to add you to other channels.', { exact: true })
    ).toBeInTheDocument();
    // The sidebar already says how other channels appear; the intro does not repeat it.
    expect(screen.queryByText(/Only channels you’ve been added to/)).toBeNull();
  });

  it('names the host, not the channel’s owner, and tells the host nothing', () => {
    const member = makeController({
      snapshot: snapshotFor({
        actor: { id: ID.bob, uid: 1001, username: 'bob', nickname: 'Bob Lee' },
      }),
      channel: { ...channel, owner_id: ID.carol, created_by: ID.carol },
    });
    const { unmount } = renderWithController(<Timeline />, member);
    expect(screen.getByText('Ask @alice to add you to other channels.')).toBeInTheDocument();
    unmount();
    renderWithController(<Timeline />, makeController());
    expect(screen.getByText('Welcome to #general')).toBeInTheDocument();
    expect(screen.queryByText(/add you to other channels/)).toBeNull();
  });

  it('names no host it cannot name, and never an ID', () => {
    const member = makeController({
      snapshot: snapshotFor({
        workspace: { id: ID.workspace, host_uid: 4242, mode: 'private', policy_epoch: 1 },
        actor: { id: ID.bob, uid: 1001, username: 'bob', nickname: 'Bob Lee' },
      }),
      channel: { ...channel, owner_id: ID.gone },
    });
    renderWithController(<Timeline />, member);
    expect(screen.getByText('Welcome to #general')).toBeInTheDocument();
    expect(screen.queryByText(/add you to other channels/)).toBeNull();
    expect(timelineRoot().innerHTML).not.toMatch(MACHINE_STRING);
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
  it('is a focusable log named for the channel, silent and busy until its messages have arrived, then polite (as a log is)', () => {
    vi.useFakeTimers();
    renderWithController(<Timeline />, makeController({ messages: [message()] }));
    const log = screen.getByRole('log', { name: 'general messages' });
    expect(log).toHaveAttribute('tabindex', '0');
    // The opening streams in without being read out, message by message (T-56).
    expect(log).toHaveAttribute('aria-live', 'off');
    expect(log).toHaveAttribute('aria-busy', 'true');
    openFully();
    expect(log).not.toHaveAttribute('aria-busy');
    // Polite once a frame has been drawn with the opening in it, not in the same commit. Polite
    // is the log role's own default, so the attribute goes rather than reading "polite": a modal's
    // `hideOthers` keeps any element carrying `aria-live` in the tree behind it (Q2-13).
    expect(log).toHaveAttribute('aria-live', 'off');
    act(() => {
      vi.advanceTimersByTime(64);
    });
    expect(log).not.toHaveAttribute('aria-live');
    expect(timelineRoot().querySelector('.biorouter-scroll-fade-top')).not.toBeNull();
  });

  it('describes its keyboard model, which nothing on screen shows (T-64)', () => {
    renderWithController(<Timeline />, makeController({ messages: [message()] }));
    const log = screen.getByRole('log', { name: 'general messages' });
    expect(log).toHaveAttribute('aria-description', timelineCopy.logDescription);
    expect(timelineCopy.logDescription).toMatch(/Up and Down/);
    expect(timelineCopy.logDescription).toMatch(/Tab reaches the message’s actions/);
  });

  it('names every row by its author and its own time, with the full date for a screen reader', () => {
    const messages = [
      message({ id: 'a', body: 'Counts are in.', at: new Date(2026, 8, 22, 10, 2) }),
      message({ id: 'b', body: 'Plot next?', at: new Date(2026, 8, 22, 10, 3) }),
    ];
    renderWithController(<Timeline />, makeController({ messages }));
    const [head, continuation] = screen
      .getAllByRole('group')
      .filter((node) => node.hasAttribute('data-crew-row'));
    expect(head).toHaveAccessibleName('Bob Lee @bob 10:02 AM, Tuesday, September 22, 2026');
    // A continuation has no author of its own on screen: it is named by the head's author.
    expect(continuation).toHaveAccessibleName('Bob Lee @bob 10:03 AM, Tuesday, September 22, 2026');
    expect(within(continuation).getByText('Plot next?')).toBeInTheDocument();
    // The date is in the time itself, not only in a hover tooltip.
    const time = within(head).getAllByText('10:02 AM')[0].closest('time') as HTMLElement;
    expect(time.querySelector('.sr-only')).toHaveTextContent(', Tuesday, September 22, 2026');
  });

  it('names each row’s actions for its message, so no two read the same (T-56)', () => {
    const messages = [
      message({ id: 'a', body: 'Counts are in.', at: new Date(2026, 8, 22, 10, 2) }),
      message({ id: 'b', body: 'Plot next?', at: new Date(2026, 8, 22, 10, 3) }),
      message({
        id: 'c',
        body: 'Done.',
        actor_id: ID.carol,
        at: new Date(2026, 8, 22, 10, 4),
      }),
    ];
    renderWithController(<Timeline />, makeController({ messages }));
    const copies = screen
      .getAllByRole('button', { name: /^Copy text/, hidden: true })
      .map((button) => button.getAttribute('aria-label'));
    expect(copies).toEqual([
      timelineCopy.copyTextOf('Bob Lee', '10:02 AM'),
      timelineCopy.copyTextOf('Bob Lee', '10:03 AM'),
      timelineCopy.copyTextOf('@carol', '10:04 AM'),
    ]);
    const more = screen
      .getAllByRole('button', { name: /^More actions/, hidden: true })
      .map((button) => button.getAttribute('aria-label'));
    expect(new Set(more).size).toBe(3);
    expect(more[0]).toBe(timelineCopy.moreActionsFor('Bob Lee', '10:02 AM'));
  });

  it('tells two rows of one minute apart in their action names (Q2-57)', () => {
    const messages = [
      message({ id: 'a', body: 'Counts are in.', at: new Date(2026, 8, 22, 10, 2, 5) }),
      message({ id: 'b', body: 'Plot next?', at: new Date(2026, 8, 22, 10, 2, 40) }),
    ];
    renderWithController(<Timeline />, makeController({ messages }));
    const copies = screen
      .getAllByRole('button', { name: /^Copy text/, hidden: true })
      .map((button) => button.getAttribute('aria-label'));
    expect(copies).toEqual([
      'Copy text of Bob Lee’s message, 10:02 AM, 1 of 2',
      'Copy text of Bob Lee’s message, 10:02 AM, 2 of 2',
    ]);
    const more = screen
      .getAllByRole('button', { name: /^More actions/, hidden: true })
      .map((button) => button.getAttribute('aria-label'));
    expect(more).toEqual([
      'More actions for Bob Lee’s message, 10:02 AM, 1 of 2',
      'More actions for Bob Lee’s message, 10:02 AM, 2 of 2',
    ]);
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
    // One initial at every avatar size (Q3-62).
    expect(human.querySelector('[data-slot="avatar"]')).toHaveTextContent(/^B$/);
  });

  it('names a former author from the names their messages came with, not as “Unknown member”', () => {
    renderWithController(
      <Timeline />,
      makeController({
        messages: [message({ actor_id: ID.gone, body: 'before I left' })],
        people: { [ID.gone]: { username: 'dan', display_name: 'Dan Wu', active: false } },
      })
    );
    const article = screen.getByRole('article');
    expect(article).toHaveTextContent('Dan Wu');
    expect(article).toHaveTextContent(`· ${identityCopy.formerMember}`);
    expect(article).not.toHaveTextContent(identityCopy.unknownMember);
    expect(timelineRoot().innerHTML).not.toContain(ID.gone);
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
    // The first unread message is its day's first: "New" rides the day's band, in accent.
    const band = document.querySelector<HTMLElement>(".crew-day-label[data-new='true']")!;
    expect(band).toHaveAttribute('role', 'separator');
    expect(within(band).getByText(timelineCopy.newLine)).toHaveClass('text-text-accent');
    expect(band.outerHTML).not.toMatch(/danger/);
    // The line sits right before the unread message.
    const fresh = screen.getByText('fresh');
    expect(newLine.compareDocumentPosition(fresh) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('draws the New line in accent mid-day, before the first unread message', () => {
    const messages = [
      message({ id: 'a', sequence: 's1', at: new Date(2026, 8, 22, 9, 0) }),
      message({ id: 'b', sequence: 's2', at: new Date(2026, 8, 22, 9, 30), body: 'fresh' }),
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
    const newLine = screen.getByRole('separator', { name: timelineCopy.newLineLabel });
    expect(within(newLine).getByText(timelineCopy.newLine)).toHaveClass('text-text-accent');
    expect(newLine.outerHTML).not.toMatch(/danger/);
    const fresh = screen.getByText('fresh');
    expect(newLine.compareDocumentPosition(fresh) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('puts “New” on the day’s rule when the first unread message is the day’s first (Q2-53)', () => {
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
    // One line, not two 30px apart: the day's own band, its hairline in accent with "New" at its
    // end. A screen reader meets "New messages" once, right before the day.
    const band = document.querySelector<HTMLElement>(".crew-day-label[data-new='true']")!;
    expect(band).toHaveAttribute('role', 'separator');
    expect(band).toHaveAccessibleName(/^(Today|Yesterday|\w+day, September 22)$/);
    expect(within(band).getByText(timelineCopy.newLine)).toHaveClass('text-text-accent');
    expect(document.querySelector('.crew-new-divider')).toBeNull();
    expect(screen.getAllByRole('separator', { name: timelineCopy.newLineLabel })).toHaveLength(1);
    const newLine = screen.getByRole('separator', { name: timelineCopy.newLineLabel });
    expect(newLine.compareDocumentPosition(band) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
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

  it('tells the attachments slot whether its row is the active one, so its controls are Tab stops only then (Q3-05)', () => {
    const renderAttachments = vi.fn((item: CrewMessage, slot: AttachmentSlotState) => (
      <button type="button" tabIndex={slot.active ? 0 : -1}>{`Save ${item.body}`}</button>
    ));
    const messages = [
      message({ id: 'f1', body: 'one.csv', attachments: ['b1'] }),
      message({ id: 'f2', body: 'two.csv', actor_id: ID.carol, attachments: ['b2'] }),
    ];
    renderWithController(
      <Timeline renderAttachments={renderAttachments} />,
      makeController({ messages })
    );
    const save = (name: string) => screen.getByRole('button', { name: `Save ${name}` });
    expect(save('one.csv').tabIndex).toBe(-1);
    expect(save('two.csv').tabIndex).toBe(-1);

    const log = screen.getByRole('log');
    act(() => log.focus());
    fireEvent.keyDown(log, { key: 'ArrowUp' });
    expect(save('two.csv').tabIndex).toBe(0);
    expect(save('one.csv').tabIndex).toBe(-1);

    // A pointer focus on a card's control makes its row the active one.
    act(() => save('one.csv').focus());
    expect(save('one.csv').tabIndex).toBe(0);
    expect(save('two.csv').tabIndex).toBe(-1);
  });

  describe('the viewer’s own agent, posting from one of their chats (Q3-22)', () => {
    const chats = new Map([[ID.run, { title: 'Assay results summary', sessionId: ID.session }]]);

    it('heads its posts “Your agent · {chat title}”, the title opening that chat', async () => {
      const messages = [
        message({ id: 'mine', actor_id: ID.alice, run_id: ID.run, body: 'Summary posted.' }),
      ];
      renderWithController(<Timeline ownAgentChats={chats} />, makeController({ messages }));
      const [post] = screen.getAllByRole('article');
      expect(post).toHaveTextContent(
        `${identityCopy.yourAgent} · Assay results summary${timelineCopy.agentBadge}`
      );
      expect(post).not.toHaveTextContent('@alice');
      const open = within(post).getByRole('button', { name: 'Assay results summary' });
      // In the Tab order only on the active row, like the row's other controls.
      expect(open.tabIndex).toBe(-1);
      await userEvent.setup(pointerAnywhere).click(open);
      expect(screen.getByTestId('location')).toHaveTextContent(
        `/pair?resumeSessionId=${encodeURIComponent(ID.session)}`
      );
      // The row is still named by who posted and when.
      expect(
        screen.getByRole('group', { name: /^Your agent · Assay results summary/ })
      ).toBeInTheDocument();
    });

    it('never names another person’s agent by a chat, even one listed under its run', () => {
      // A run ID that another person's agent posted under can never be one of the viewer's
      // grants; if a map held it anyway, the head would still read as that person's agent.
      const messages = [
        message({ id: 'theirs', actor_id: ID.bob, run_id: ID.run, body: 'Plot attached.' }),
        message({ id: 'task', actor_id: ID.alice, run_id: ID.runB, body: 'Task: plot' }),
      ];
      renderWithController(<Timeline ownAgentChats={chats} />, makeController({ messages }));
      const [theirs, task] = screen.getAllByRole('article');
      expect(theirs).toHaveTextContent(`Bob Lee's agent @bob${timelineCopy.agentBadge}`);
      expect(theirs).not.toHaveTextContent('Assay results summary');
      // The viewer's agent under a run with no titled chat keeps the plain head.
      expect(task).toHaveTextContent(`${identityCopy.yourAgent} @alice${timelineCopy.agentBadge}`);
    });
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
  it('hands its consumers one copy action for its whole life', () => {
    // A new action on every render re-rendered every row's actions on every
    // message and every keystroke in the composer.
    const seen = new Set<unknown>();
    function Probe() {
      seen.add(useTimelineCopy());
      return null;
    }
    const { rerender } = render(
      <TimelineCopyProvider>
        <Probe />
      </TimelineCopyProvider>
    );
    rerender(
      <TimelineCopyProvider>
        <Probe />
      </TimelineCopyProvider>
    );
    expect(seen.size).toBe(1);
  });

  /** user-event installs its own clipboard on setup, so spy on the one it installed. */
  function setupWithClipboard() {
    const user = userEvent.setup(pointerAnywhere);
    const writeText = vi.spyOn(navigator.clipboard, 'writeText');
    return { user, writeText };
  }

  const copyButton = () => screen.getByRole('button', { name: /^Copy text of Bob Lee’s message/ });
  const moreButton = () => screen.getByRole('button', { name: /^More actions for Bob Lee’s/ });
  const liveStatus = () =>
    screen.getAllByRole('status').find((node) => node.getAttribute('aria-live') === 'polite');
  /** Into ⋯'s "Copy for support" submenu, by keyboard, as a person would. */
  async function openSupport(user: ReturnType<typeof userEvent.setup>) {
    const support = await screen.findByRole('menuitem', { name: timelineCopy.copyForSupport });
    act(() => support.focus());
    await user.keyboard('{ArrowRight}');
  }

  it('copies the text and, from ⋯, the message ID, and says so without a toast', async () => {
    renderWithController(
      <Timeline />,
      makeController({ messages: [message({ id: 'msg-7', body: 'Counts are **in**.' })] })
    );
    const { user, writeText } = setupWithClipboard();
    await user.click(copyButton());
    expect(writeText).toHaveBeenCalledWith('Counts are **in**.');
    await waitFor(() => expect(liveStatus()).toHaveTextContent(timelineCopy.copied));

    await user.click(moreButton());
    await openSupport(user);
    await user.click(await screen.findByRole('menuitem', { name: timelineCopy.copyMessageId }));
    expect(writeText).toHaveBeenLastCalledWith('msg-7');
    expect(document.querySelector('.Toastify')).toBeNull();
  });

  it('answers on the control itself: “Copied” with a check, for two seconds', async () => {
    renderWithController(<Timeline />, makeController({ messages: [message()] }));
    const { user } = setupWithClipboard();
    await user.click(copyButton());
    await waitFor(() => expect(copyButton()).toHaveAttribute('data-copy-outcome', 'copied'));
    expect(await screen.findByRole('tooltip')).toHaveTextContent(timelineCopy.copied);
  });

  it('says how to copy by hand when the clipboard refuses, and “Couldn’t copy” on the control', async () => {
    renderWithController(<Timeline />, makeController({ messages: [message()] }));
    const { user, writeText } = setupWithClipboard();
    writeText.mockRejectedValueOnce(new Error('denied'));
    await user.click(copyButton());
    await waitFor(() => expect(liveStatus()).toHaveTextContent(timelineCopy.copyFailed));
    // Never silent where the press happened (T-56).
    expect(copyButton()).toHaveAttribute('data-copy-outcome', 'failed');
    expect(await screen.findByRole('tooltip')).toHaveTextContent(timelineCopy.copyFailedShort);
  });

  it('answers a copy from ⋯ in the menu itself, then closes it (Q2-34)', async () => {
    renderWithController(
      <Timeline />,
      makeController({ messages: [message({ id: 'msg-9', body: 'Plot it.' })] })
    );
    const { user, writeText } = setupWithClipboard();
    await user.click(moreButton());
    await openSupport(user);
    await user.click(await screen.findByRole('menuitem', { name: timelineCopy.copyMessageId }));
    expect(writeText).toHaveBeenLastCalledWith('msg-9');
    const item = await screen.findByRole('menuitem', { name: timelineCopy.copied });
    expect(item).toHaveAttribute('data-crew-copy-state', 'copied');
    await waitFor(() => expect(liveStatus()).toHaveTextContent(timelineCopy.copied));
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), { timeout: 2000 });
  });

  it('never gives ⋯ a menu that holds only the message ID, and keeps the ID in “Copy for support” (Q3-26)', async () => {
    renderWithController(
      <Timeline />,
      makeController({ messages: [message({ id: 'msg-8', body: 'Plot it.' })] })
    );
    const { user, writeText } = setupWithClipboard();
    await user.click(moreButton());
    const menu = await screen.findByRole('menu');
    const items = Array.from(menu.querySelectorAll('[role="menuitem"], [role="separator"]')).map(
      (node) => (node.getAttribute('role') === 'separator' ? '—' : node.textContent)
    );
    // The person's copy first; the machine string last, after a separator, one step away.
    expect(items).toEqual([timelineCopy.copyText, '—', timelineCopy.copyForSupport]);
    expect(screen.getByRole('menuitem', { name: timelineCopy.copyForSupport })).toHaveAttribute(
      'aria-haspopup',
      'menu'
    );
    await user.click(screen.getByRole('menuitem', { name: timelineCopy.copyText }));
    expect(writeText).toHaveBeenLastCalledWith('Plot it.');
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
      messages: [...live, postedNow({ id: 'after', body: 'just posted' })],
      historyBefore: null,
    });
    expect(screen.getByText('just posted').closest('[data-crew-row]')).toHaveAttribute(
      'data-arriving',
      'true'
    );
    expect(arrivingRows()).toBe(1);
  });

  it('measures a full page by the size the observer asks for', () => {
    const controller = makeController({ messages: page(50), pageSize: 50 });
    renderWithController(<Timeline />, controller);
    expect(screen.getByRole('button', { name: 'Older messages' })).toBeInTheDocument();
    expect(screen.queryByText('Welcome to #general')).toBeNull();
    // The same list under a full-size page is the whole channel.
    renderWithController(<Timeline />, makeController({ messages: page(50), pageSize: 200 }));
    expect(screen.getByText('Welcome to #general')).toBeInTheDocument();
  });

  it('marks the log busy while a page loads', () => {
    renderWithController(
      <Timeline />,
      makeController({ messages: [], messagesLoaded: false, historyBefore: 's100' })
    );
    expect(screen.getByRole('log')).toHaveAttribute('aria-busy', 'true');
    expect(screen.getByText(timelineCopy.loadingOlder)).toBeInTheDocument();
  });

  it('keeps an older page out of the live announcements while it lands (T-56)', async () => {
    const live = page(HISTORY_PAGE_SIZE);
    const controller = makeController({ messages: live });
    const view = renderWithController(<Timeline />, controller);
    const log = screen.getByRole('log');
    await waitFor(() => expect(log).not.toHaveAttribute('aria-live'));
    expect(log).not.toHaveAttribute('aria-busy');

    // The boundary moves first, with the previous page still drawn: busy and silent.
    view.rerenderWith({ ...controller, historyBefore: live[0].sequence });
    expect(log).toHaveAttribute('aria-busy', 'true');
    expect(log).toHaveAttribute('aria-live', 'off');

    // The list is cleared, then the older page lands: still silent while it goes in.
    view.rerenderWith({
      ...controller,
      historyBefore: live[0].sequence,
      messages: [],
      messagesLoaded: false,
    });
    expect(log).toHaveAttribute('aria-live', 'off');
    const older = Array.from({ length: 5 }, (_, index) =>
      message({ id: `o-${index}`, body: `older ${index}`, at: new Date(2026, 8, 21, 9, index) })
    );
    const seen: MutationRecord[] = [];
    const mutations = new MutationObserver((records) => seen.push(...records));
    mutations.observe(log, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ['aria-live'],
      attributeOldValue: true,
    });
    view.rerenderWith({ ...controller, historyBefore: live[0].sequence, messages: older });
    seen.push(...mutations.takeRecords());
    const landed = seen.length;
    // The rows went in with the log off, and it stays off for the frame that draws them.
    expect(seen.some((record) => record.addedNodes.length > 0)).toBe(true);
    expect(log).toHaveAttribute('aria-live', 'off');
    expect(log).not.toHaveAttribute('aria-busy');
    await waitFor(() => expect(log).not.toHaveAttribute('aria-live'));
    seen.push(...mutations.takeRecords());
    mutations.disconnect();
    // Polite again only after that frame, in a change that inserts nothing of its own.
    const flipped = seen.slice(landed);
    expect(flipped.some((record) => record.oldValue === 'off')).toBe(true);
    expect(flipped.some((record) => record.addedNodes.length > 0)).toBe(false);
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

describe('a post on its way (T-37)', () => {
  const sent = 'Hi all, the counts are in.';
  const draft = (body: string) => ({ body, attachments: [], references: [] });
  const before = message({ id: 'm-before', body: 'Morning.' });

  /** The controller as the composer drives it: the draft, then the post in flight, then its answer. */
  function stages() {
    const idle = makeController({ messages: [before], draft: draft(sent) });
    const posting = { ...idle, isPending: vi.fn((key: string) => key === 'send') };
    const accepted = { ...idle, draft: draft(''), isPending: vi.fn(() => false) };
    return { idle, posting, accepted };
  }
  const sending = () => screen.queryByText(timelineCopy.sending);

  it('shows the words that left the composer as “Sending…” until the observer delivers them', () => {
    const { idle, posting, accepted } = stages();
    const view = renderWithController(<Timeline />, idle);
    view.rerenderWith(posting);
    // While the broker has not answered, the composer still holds the words and shows its spinner.
    expect(sending()).toBeNull();
    view.rerenderWith(accepted);
    expect(sending()).toBeInTheDocument();
    const row = sending()?.closest('.crew-pending-row') as HTMLElement;
    expect(row).toHaveTextContent(sent);
    // Not a message yet: out of the log's announcements, the row keys and the tab order.
    expect(row).toHaveAttribute('aria-hidden', 'true');
    expect(row).not.toHaveAttribute('data-crew-row');
    expect(row.hasAttribute('inert')).toBe(true);

    view.rerenderWith({
      ...accepted,
      messages: [before, message({ id: 'm-sent', actor_id: ID.alice, body: sent })],
    });
    expect(sending()).toBeNull();
    // Delivered in the same render the stand-in goes: the words are never on screen twice.
    expect(screen.getAllByText(sent)).toHaveLength(1);
  });

  it('shows who is sending: the viewer’s avatar and name, not under the previous person (Q3-20)', () => {
    const { idle, posting, accepted } = stages();
    const view = renderWithController(<Timeline />, idle);
    view.rerenderWith(posting);
    view.rerenderWith(accepted);
    const row = sending()?.closest('.crew-pending-row') as HTMLElement;
    // "Morning." is Bob's; the post on its way heads a group of its own, as Alice.
    expect(row).toHaveAttribute('data-head', 'true');
    expect(row).toHaveTextContent('Alice Chen');
    // Her own avatar: one initial, as every avatar (Q3-62), in a circle beside her name.
    expect(row.querySelector('[data-slot="avatar"]')).toHaveAttribute('data-shape', 'circle');
    expect(row.querySelector('[data-slot="avatar"]')).toHaveTextContent('A');
  });

  it('continues the viewer’s own group without a second head, as the message will', () => {
    const mine = message({ id: 'm-mine', actor_id: ID.alice, body: 'First.', at: new Date() });
    const idle = makeController({ messages: [mine], draft: draft(sent) });
    const view = renderWithController(<Timeline />, idle);
    view.rerenderWith({ ...idle, isPending: vi.fn((key: string) => key === 'send') });
    view.rerenderWith({ ...idle, draft: draft(''), isPending: vi.fn(() => false) });
    const row = sending()?.closest('.crew-pending-row') as HTMLElement;
    expect(row).not.toHaveAttribute('data-head');
    expect(row).not.toHaveTextContent('Alice Chen');
  });

  it('shows nothing for a post the broker refused: the composer keeps the words and says why', () => {
    const { idle, posting } = stages();
    const view = renderWithController(<Timeline />, idle);
    view.rerenderWith(posting);
    view.rerenderWith({
      ...idle,
      error: { message: 'Not allowed.', source: 'composer' },
      isPending: vi.fn(() => false),
    });
    expect(sending()).toBeNull();
  });

  it('takes a draft cleared by a dropped verified view for no answer at all', () => {
    // As the layout mounts it: while Crew re-verifies, the last verified view, read-only.
    const lastView = {
      snapshot: snapshotFor(),
      channel,
      messages: [before],
      messagesLoaded: true,
      runs: [],
      labels: null,
      historyBefore: null,
    };
    function Stage() {
      const crew = useCrew();
      return <Timeline view={crew.snapshot ? null : lastView} readOnly={!crew.snapshot} />;
    }
    const { idle, posting, accepted } = stages();
    const view = renderWithController(<Stage />, idle);
    view.rerenderWith(posting);
    view.rerenderWith({ ...accepted, snapshot: null });
    view.rerenderWith(accepted);
    expect(screen.getByText('Morning.')).toBeInTheDocument();
    expect(sending()).toBeNull();
  });

  it('is not fooled by someone else posting the same words', () => {
    const { idle, posting, accepted } = stages();
    const view = renderWithController(<Timeline />, idle);
    view.rerenderWith(posting);
    view.rerenderWith(accepted);
    view.rerenderWith({
      ...accepted,
      messages: [before, message({ id: 'm-bob', actor_id: ID.bob, body: sent })],
    });
    expect(sending()).toBeInTheDocument();
  });

  it('goes quietly if the message never arrives', () => {
    vi.useFakeTimers();
    const { idle, posting, accepted } = stages();
    const view = renderWithController(<Timeline />, idle);
    view.rerenderWith(posting);
    view.rerenderWith(accepted);
    expect(sending()).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(PENDING_POST_TIMEOUT_MS);
    });
    expect(sending()).toBeNull();
  });

  it('draws nothing in a read-only view', () => {
    const { idle, posting, accepted } = stages();
    const view = renderWithController(<Timeline readOnly />, idle);
    view.rerenderWith(posting);
    view.rerenderWith(accepted);
    expect(sending()).toBeNull();
  });
});

describe('arrivals', () => {
  it('lets a message that arrives while following rise in, and nothing that was already there', () => {
    vi.useFakeTimers();
    const first = [message({ id: 'old', body: 'old' })];
    const controller = makeController({ messages: first });
    const { rerenderWith } = renderWithController(<Timeline />, controller);
    openFully();
    const oldRow = screen.getByText('old').closest('[data-crew-row]');
    expect(oldRow).not.toHaveAttribute('data-arriving');

    rerenderWith({ ...controller, messages: [...first, postedNow({ id: 'new', body: 'new' })] });
    expect(screen.getByText('new').closest('[data-crew-row]')).toHaveAttribute(
      'data-arriving',
      'true'
    );
    expect(screen.getByText('old').closest('[data-crew-row]')).not.toHaveAttribute('data-arriving');
  });
});

describe('a channel streaming in, one message per frame', () => {
  // The daemon's observer sends the live tail oldest first, ONE message per
  // frame (routes/crew_observation.rs), each behind several broker round trips,
  // and useCrewObservation marks the list loaded on the first. Nothing that
  // describes the whole channel may be decided from the first frames.
  beforeEach(() => {
    vi.useFakeTimers();
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  const tailOf = (count: number) =>
    Array.from({ length: count }, (_, index) =>
      message({
        id: `t-${index}`,
        body: `tail ${index}`,
        actor_id: index % 2 ? ID.bob : ID.carol,
        at: new Date(2026, 8, 22, 6, 0, index * 20),
      })
    );
  // Class lookups, not selectors or role queries: these run on every one of two
  // hundred frames, over a list two hundred rows long.
  const arrivingRows = () => document.querySelectorAll('[data-arriving="true"]').length;
  const byClass = (name: string) => document.getElementsByClassName(name);
  const shownIntro = () => {
    const intro = byClass('crew-channel-intro')[0];
    return intro && !intro.hasAttribute('data-pending') ? intro : null;
  };
  const newLine = () => byClass('crew-new-divider')[0] ?? null;
  const before = (first: Node, second: Node) =>
    Boolean(first.compareDocumentPosition(second) & Node.DOCUMENT_POSITION_FOLLOWING);
  const row = (body: string) => {
    const exact = new RegExp(`${body}(?!\\d)`);
    const found = Array.from(document.querySelectorAll('[data-crew-row]')).find((element) =>
      exact.test(element.textContent ?? '')
    );
    if (!found) throw new Error(`no row reads ${body}`);
    return found;
  };
  const busy = () => byClass('crew-timeline-log')[0]?.getAttribute('aria-busy') ?? null;

  /**
   * Opens the channel with nothing loaded, then delivers `tail` one message per
   * render, `gap` apart, calling `check` with the index of the newest message.
   */
  function stream(
    tail: CrewMessage[],
    controller: ReturnType<typeof makeController>,
    gap: number,
    check: (newest: number) => void
  ) {
    const opened = { ...controller, messages: [], messagesLoaded: false };
    const view = renderWithController(<Timeline />, opened);
    tail.forEach((_, index) => {
      act(() => {
        vi.advanceTimersByTime(gap);
      });
      view.rerenderWith({ ...controller, messages: tail.slice(0, index + 1) });
      check(index);
    });
    return view;
  }

  it('puts the New line where the whole tail does, never above the first message to arrive', () => {
    // A busy channel: the tail is a full page, read up to message 150 of it,
    // with the 49 after it unread — the read position arrives 151 frames in.
    const tail = tailOf(HISTORY_PAGE_SIZE);
    const controller = makeController({
      snapshot: snapshotFor({
        read_positions: { [ID.general]: tail[150].sequence },
        unread: { [ID.general]: 49 },
      }),
    });
    // Frames slower than the mark-read dwell, and faster than the quiet window.
    const { rerenderWith } = stream(tail, controller, AUTO_READ_DWELL_MS + 100, (newest) => {
      const line = newLine();
      if (newest <= 150) expect(line).toBeNull();
      else {
        expect(line).not.toBeNull();
        expect(before(row('tail 150') as Node, line as Node)).toBe(true);
        expect(before(line as Node, row('tail 151') as Node)).toBe(true);
      }
      // No false "start of the channel" while the page is still filling.
      expect(shownIntro()).toBeNull();
      if (newest < HISTORY_PAGE_SIZE - 1) {
        expect(busy()).toBe('true');
        expect(byClass('crew-history-sentinel')).toHaveLength(0);
        // Not marked read to a message in the middle of the backlog.
        expect(controller.markRead).not.toHaveBeenCalled();
      }
    });

    // Nothing posted before the channel opened rose in as an arrival. A row keeps
    // its mark once given (the set only grows), so one look covers every frame.
    expect(arrivingRows()).toBe(0);
    // The full page is in: "Older messages", no intro, not busy, one New line.
    expect(screen.getByRole('button', { name: timelineCopy.older })).toBeInTheDocument();
    expect(document.querySelector('.crew-channel-intro')).toBeNull();
    expect(busy()).toBeNull();
    expect(document.querySelectorAll('.crew-new-divider')).toHaveLength(1);
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS);
    });
    expect(controller.markRead).toHaveBeenCalledTimes(1);
    expect(controller.markRead).toHaveBeenCalledWith(ID.general, tail[199].sequence);

    // A message posted after the channel opened is a live arrival.
    rerenderWith({
      ...controller,
      messages: [...tail.slice(1), postedNow({ id: 'live', body: 'just posted' })],
    });
    expect(screen.getByText('just posted').closest('[data-crew-row]')).toHaveAttribute(
      'data-arriving',
      'true'
    );
    expect(arrivingRows()).toBe(1);
  });

  it('keeps the intro’s place while a short channel streams in, and shows it once the tail has arrived', () => {
    const tail = tailOf(30);
    const controller = makeController({
      snapshot: snapshotFor({
        read_positions: { [ID.general]: tail[29].sequence },
        unread: { [ID.general]: 0 },
      }),
    });
    stream(tail, controller, 200, () => {
      // The place is held from the first message, so nothing moves down later…
      expect(document.querySelector('.crew-channel-intro[data-pending="true"]')).not.toBeNull();
      // …but it claims nothing: hidden from assistive technology, and inert.
      expect(screen.queryByRole('heading', { name: 'Welcome to #general' })).toBeNull();
      expect(screen.getByRole('log')).toHaveAttribute('aria-busy', 'true');
      expect(arrivingRows()).toBe(0);
    });
    act(() => {
      vi.advanceTimersByTime(OPENING_QUIET_MS - 1);
    });
    expect(shownIntro()).toBeNull();
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(screen.getByRole('heading', { name: 'Welcome to #general' })).toBeInTheDocument();
    expect(screen.getByRole('log')).not.toHaveAttribute('aria-busy');
    // Read through its newest message: no New line, nothing to mark read.
    expect(newLine()).toBeNull();
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_MIN_INTERVAL_MS);
    });
    expect(controller.markRead).not.toHaveBeenCalled();
  });

  it('draws no New line for a channel the broker counts nothing unread in, whatever streams in', () => {
    // The viewer's agent posted after the read position: the broker counts
    // neither the viewer's posts nor their agent's, and neither does the line.
    const tail = [
      message({ id: 'r', sequence: 's1', body: 'read' }),
      message({ id: 'x', sequence: 's2', body: 'result', actor_id: ID.alice, run_id: ID.run }),
    ];
    stream(
      tail,
      makeController({
        snapshot: snapshotFor({
          read_positions: { [ID.general]: 's1' },
          unread: { [ID.general]: 0 },
        }),
      }),
      200,
      () => expect(newLine()).toBeNull()
    );
    openFully();
    expect(newLine()).toBeNull();
  });

  it('settles with what it holds when the stream stalls before the messages it expects', () => {
    // The read position never arrives (a tail the daemon shortened): after the
    // stall window the list stops being busy and the line falls back to the count.
    const tail = tailOf(5);
    const controller = makeController({
      snapshot: snapshotFor({
        read_positions: { [ID.general]: 'not-in-this-tail' },
        unread: { [ID.general]: 2 },
      }),
    });
    stream(tail, controller, 200, () => expect(newLine()).toBeNull());
    act(() => {
      vi.advanceTimersByTime(OPENING_QUIET_MS);
    });
    // Provably still streaming: the quiet window is not enough.
    expect(screen.getByRole('log')).toHaveAttribute('aria-busy', 'true');
    act(() => {
      vi.advanceTimersByTime(OPENING_STALL_MS - OPENING_QUIET_MS);
    });
    expect(screen.getByRole('log')).not.toHaveAttribute('aria-busy');
    const line = newLine();
    expect(line).not.toBeNull();
    expect(before(screen.getByText('tail 2'), line as Node)).toBe(true);
    expect(before(line as Node, screen.getByText('tail 3'))).toBe(true);
  });
});

describe('the observer’s word on the backlog', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  const short = () =>
    Array.from({ length: 5 }, (_, index) =>
      message({ id: `b-${index}`, body: `backlog ${index}`, at: new Date(2026, 8, 22, 7, index) })
    );

  it('keeps the opening open while the daemon says more is coming, and never times it out', () => {
    const tail = short();
    const controller = makeController({
      messages: tail,
      backlogComplete: false,
      snapshot: snapshotFor({
        read_positions: { [ID.general]: tail[4].sequence },
        unread: { [ID.general]: 0 },
      }),
    });
    const { rerenderWith } = renderWithController(<Timeline />, controller);
    act(() => {
      vi.advanceTimersByTime(OPENING_STALL_MS * 2);
    });
    // No quiet or stall timer decides for a daemon that counts the backlog down.
    expect(screen.getByRole('log')).toHaveAttribute('aria-busy', 'true');
    expect(screen.queryByRole('heading', { name: 'Welcome to #general' })).toBeNull();

    rerenderWith({ ...controller, backlogComplete: true });
    expect(screen.getByRole('log')).not.toHaveAttribute('aria-busy');
    expect(screen.getByRole('heading', { name: 'Welcome to #general' })).toBeInTheDocument();
  });

  it('opens at once when the daemon says the backlog is in', () => {
    renderWithController(
      <Timeline />,
      makeController({ messages: short(), backlogComplete: true })
    );
    expect(screen.getByRole('log')).not.toHaveAttribute('aria-busy');
    expect(screen.getByRole('heading', { name: 'Welcome to #general' })).toBeInTheDocument();
  });
});

describe('following a full live tail', () => {
  // jsdom lays nothing out. The scroll area's viewport is given a fixed height and content, and a
  // scrollTop that clamps, from before the timeline mounts — the scroll area measures the viewport
  // when it mounts, and a height that changed afterwards would read as a resize, not a scroll.
  const SCROLL_HEIGHT = 20_000;
  const CLIENT_HEIGHT = 600;
  const MAX_TOP = SCROLL_HEIGHT - CLIENT_HEIGHT;
  const tops = new WeakMap<Element, number>();
  const scrollTo = vi.fn();
  const isViewport = (element: Element) => element.hasAttribute('data-radix-scroll-area-viewport');
  const inherited = (name: 'scrollHeight' | 'clientHeight' | 'scrollTop') =>
    Object.getOwnPropertyDescriptor(Element.prototype, name);

  beforeEach(() => {
    vi.useFakeTimers();
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const proto = HTMLDivElement.prototype as unknown as Record<string, unknown>;
    Object.defineProperty(proto, 'scrollHeight', {
      configurable: true,
      get(this: Element) {
        return isViewport(this) ? SCROLL_HEIGHT : inherited('scrollHeight')?.get?.call(this);
      },
    });
    Object.defineProperty(proto, 'clientHeight', {
      configurable: true,
      get(this: Element) {
        return isViewport(this) ? CLIENT_HEIGHT : inherited('clientHeight')?.get?.call(this);
      },
    });
    Object.defineProperty(proto, 'scrollTop', {
      configurable: true,
      get(this: Element) {
        return isViewport(this) ? (tops.get(this) ?? 0) : inherited('scrollTop')?.get?.call(this);
      },
      set(this: Element, value: number) {
        if (isViewport(this)) tops.set(this, Math.max(0, Math.min(value, MAX_TOP)));
        else inherited('scrollTop')?.set?.call(this, value);
      },
    });
    scrollTo.mockReset();
    scrollTo.mockImplementation(function (this: HTMLElement, options?: { top?: number }) {
      if (typeof options?.top === 'number') this.scrollTop = options.top;
    });
    proto.scrollTo = scrollTo;
  });
  afterEach(() => {
    const proto = HTMLDivElement.prototype as unknown as Record<string, unknown>;
    for (const name of ['scrollHeight', 'clientHeight', 'scrollTop', 'scrollTo'])
      delete proto[name];
    vi.restoreAllMocks();
  });

  function viewport(): HTMLElement {
    const element = document.querySelector<HTMLElement>('[data-radix-scroll-area-viewport]');
    if (!element) throw new Error('no viewport');
    return element;
  }
  const fromBottom = () => MAX_TOP - viewport().scrollTop;
  /** Where the browser's scroll anchoring leaves the view: no scroll event is dispatched. */
  const place = (top: number) => tops.set(viewport(), top);

  it('keeps the newest message in view when an arrival does not grow the log, and marks read only once it is on screen', () => {
    const tail = page(HISTORY_PAGE_SIZE);
    const readAll = snapshotFor({
      read_positions: { [ID.general]: tail[HISTORY_PAGE_SIZE - 1].sequence },
      unread: { [ID.general]: 0 },
    });
    const controller = makeController({ messages: tail, snapshot: readAll });
    const { rerenderWith } = renderWithController(<Timeline />, controller);
    // The channel opened at its newest message.
    expect(fromBottom()).toBe(0);

    // The oldest message leaves as the newest arrives: the tail keeps its size, the content
    // does not grow, and scroll anchoring holds the rows on screen — one row above the bottom.
    const arrived = postedNow({ id: 'arrived', body: 'new on a full tail' });
    const next = [...tail, arrived].slice(-HISTORY_PAGE_SIZE);
    expect(next).toHaveLength(HISTORY_PAGE_SIZE);
    place(MAX_TOP - 100);
    rerenderWith({ ...controller, messages: next });
    expect(fromBottom()).toBe(0);
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS);
    });
    expect(controller.markRead).toHaveBeenCalledTimes(1);
    expect(controller.markRead).toHaveBeenCalledWith(ID.general, arrived.sequence);

    // A view that does not reach the bottom: the reader still "follows", but the newest row is
    // below the fold, so nothing is marked read however long they wait.
    scrollTo.mockImplementation(() => {});
    place(MAX_TOP - 100);
    const second = postedNow({ id: 'second', body: 'second arrival' });
    rerenderWith({ ...controller, messages: [...next, second].slice(-HISTORY_PAGE_SIZE) });
    expect(fromBottom()).toBe(100);
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_MIN_INTERVAL_MS * 3);
    });
    expect(controller.markRead).toHaveBeenCalledTimes(1);

    // The newest message comes into view: it is marked read after the next look.
    place(MAX_TOP);
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS);
    });
    expect(controller.markRead).toHaveBeenCalledTimes(2);
    expect(controller.markRead).toHaveBeenLastCalledWith(ID.general, second.sequence);
  });

  it('leaves a reader who scrolled up where they are', () => {
    const tail = page(HISTORY_PAGE_SIZE);
    const controller = makeController({ messages: tail });
    const { rerenderWith } = renderWithController(<Timeline />, controller);
    expect(fromBottom()).toBe(0);
    place(2_000);
    fireEvent.scroll(viewport());
    act(() => {
      vi.advanceTimersByTime(200);
    });
    scrollTo.mockClear();
    rerenderWith({
      ...controller,
      messages: [...tail, postedNow({ id: 'later', body: 'later' })].slice(-HISTORY_PAGE_SIZE),
    });
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_MIN_INTERVAL_MS);
    });
    expect(scrollTo).not.toHaveBeenCalled();
    expect(viewport().scrollTop).toBe(2_000);
    expect(controller.markRead).not.toHaveBeenCalled();
  });

  it('says how many messages arrived below while the reader was scrolled up (Q3-27)', () => {
    const tail = page(HISTORY_PAGE_SIZE);
    const controller = makeController({ messages: tail });
    const { rerenderWith } = renderWithController(<Timeline />, controller);
    place(2_000);
    fireEvent.scroll(viewport());
    act(() => {
      vi.advanceTimersByTime(200);
    });
    const later = [
      postedNow({ id: 'l1', body: 'one' }),
      postedNow({ id: 'l2', body: 'two', actor_id: ID.carol }),
      // The viewer's own post, and an agent's tool update, are not "new messages".
      postedNow({ id: 'l3', body: 'mine', actor_id: ID.alice }),
      postedNow({ id: 'l4', body: 'Using crew__request', actor_id: ID.bob, run_id: ID.runB }),
    ];
    rerenderWith({ ...controller, messages: [...tail, ...later].slice(-HISTORY_PAGE_SIZE) });
    const pill = screen.getByRole('button', { name: '2 new messages, jump to latest' });
    expect(pill).toHaveTextContent(timelineCopy.newMessages(2));
    expect(timelineCopy.newMessages(1)).toBe('1 new message');

    rerenderWith({
      ...controller,
      messages: [...tail, ...later, postedNow({ id: 'l5', body: 'three' })].slice(
        -HISTORY_PAGE_SIZE
      ),
    });
    expect(
      screen.getByRole('button', { name: '3 new messages, jump to latest' })
    ).toBeInTheDocument();

    // Jumping clears the count with the pill.
    fireEvent.click(screen.getByRole('button', { name: /new messages/ }));
    expect(screen.queryByRole('button', { name: /new messages/ })).toBeNull();
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
    openFully();
    expect(controller.markRead).not.toHaveBeenCalled();
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
    openFully();
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
    openFully();
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
    const copyButtons = () => screen.getAllByRole('button', { name: /^Copy text of / });
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

describe('the Tab order through a channel (Q2-12)', () => {
  const TABLE = '| Sample | od600_t0 |\n|---|---|\n| WT-1 | 0.05 |';
  const tables = [
    message({ id: 't1', body: TABLE, at: new Date(2026, 8, 22, 10, 0) }),
    message({ id: 't2', body: TABLE, actor_id: ID.carol, at: new Date(2026, 8, 22, 10, 1) }),
    message({ id: 't3', body: TABLE, at: new Date(2026, 8, 22, 10, 9) }),
  ];

  const channelTitle = () =>
    screen.getByRole('button', { name: channelHeaderCopy.menuName('general') });

  async function tabsToComposer(user: ReturnType<typeof userEvent.setup>) {
    const composer = screen.getByRole('textbox', { name: 'Message #general' });
    let stops = 0;
    while (document.activeElement !== composer && stops < 20) {
      await user.tab();
      stops += 1;
    }
    expect(document.activeElement).toBe(composer);
    return stops;
  }

  it('goes from the header to the composer in at most five stops past three tables that fit', async () => {
    renderWithController(
      <>
        <ChannelHeader />
        <Timeline />
        <Composer />
      </>,
      makeController({ messages: tables })
    );
    const user = userEvent.setup(pointerAnywhere);
    // From the header's first stop, the channel's name: its badge, the member stack, the details
    // toggle, the log, then the composer. Every table used to add a stop of its own.
    act(() => channelTitle().focus());
    expect(await tabsToComposer(user)).toBeLessThanOrEqual(5);
    // A table that fits is not a stop of its own.
    expect(document.querySelectorAll('.crew-md-table-scroll[tabindex]')).toHaveLength(0);
  });

  it('takes a used row’s actions back out of the Tab order once focus leaves the log', async () => {
    renderWithController(
      <>
        <ChannelHeader />
        <Timeline />
        <Composer />
      </>,
      makeController({ messages: tables })
    );
    const user = userEvent.setup(pointerAnywhere);
    const log = screen.getByRole('log');
    const copyButtons = () => screen.getAllByRole('button', { name: /^Copy text of / });
    // A person arrows onto a message: its actions join the Tab order…
    act(() => log.focus());
    fireEvent.keyDown(log, { key: 'ArrowUp' });
    expect(copyButtons().some((button) => button.tabIndex === 0)).toBe(true);
    // …and focus moves on, out of the log.
    act(() => channelTitle().focus());
    expect(copyButtons().every((button) => button.tabIndex === -1)).toBe(true);
    // So a Tab and some typing can never land on Copy text again.
    expect(await tabsToComposer(user)).toBeLessThanOrEqual(5);
  });

  it('keeps the row’s actions in the Tab order while focus moves among them', () => {
    renderWithController(<Timeline />, makeController({ messages: tables }));
    const log = screen.getByRole('log');
    act(() => log.focus());
    fireEvent.keyDown(log, { key: 'ArrowUp' });
    const active = screen
      .getAllByRole('button', { name: /^Copy text of / })
      .find((button) => button.tabIndex === 0) as HTMLElement;
    act(() => active.focus());
    expect(active.tabIndex).toBe(0);
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
      const row = screen.getByRole('group', { name: /^Your agent · / });
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

describe('the stylesheet (what jsdom cannot lay out)', () => {
  const css = readFileSync(join(__dirname, 'timeline.css'), 'utf8').replace(
    /\/\*[\s\S]*?\*\//g,
    ''
  );
  const rule = (selector: string) => {
    const at = css.indexOf(`${selector} {`);
    if (at < 0) throw new Error(`no rule for ${selector}`);
    return css.slice(at, css.indexOf('}', at));
  };

  it('keeps the hover toolbar off the New label and off the day band above it (T-56)', () => {
    // Both selectors share one rule; the stylesheet may wrap them across lines.
    const flat = css.replace(/\s+/g, ' ');
    expect(flat).toContain(
      '.crew-new-divider + .crew-message-group > .crew-message-row:first-child > .crew-row-actions, .crew-day-label + .crew-message-group > .crew-message-row:first-child > .crew-row-actions { top: 4px; }'
    );
  });

  it('rides the day on a full-width band that hides a line whole, never a pill over it (Q3-19)', () => {
    const band = rule('.crew-day-label');
    expect(band).toMatch(/position: sticky;/);
    expect(band).toMatch(/top: 0;/);
    expect(band).toMatch(/height: 28px;/);
    // The column's own 16px gutters too, so no word is cut at the band's edge.
    expect(band).toMatch(/margin: 10px -16px 4px;/);
    expect(band).toMatch(/background-color: var\(--background-default\);/);
    expect(band).toMatch(/border-bottom: 1px solid var\(--border-subtle\);/);
    // The scroller's top fade takes only the band's padding, never the 28px with the name.
    expect(band).toMatch(/padding: var\(--scroll-fade-top\) 16px 0;/);
    expect(rule('.crew-day-pill')).not.toMatch(/box-shadow|border-radius/);
    // Forced colours keep the band's ground and its line.
    const forced = css.slice(css.indexOf('@media (forced-colors: active)'));
    expect(forced).toMatch(
      /\.crew-day-label \{\s*border-bottom: 1px solid CanvasText;\s*background-color: Canvas;/
    );
    // The New line stays in the flow; only the day's band rides the top.
    expect(rule('.crew-new-divider')).not.toMatch(/sticky/);
    expect(rule(".crew-day-label[data-new='true']")).toMatch(/var\(--accent-bar\)/);
  });

  it('rings a focused row, keeping its fill: the fill alone was 1.3:1 (Q3-21)', () => {
    const focused = rule('.crew-message-row:focus-visible');
    expect(focused).toMatch(/outline: 2px solid var\(--ring\);/);
    expect(focused).toMatch(/outline-offset: -2px;/);
    expect(focused).toMatch(/background-color: var\(--background-focus\);/);
    expect(rule('.crew-task-row:focus-visible')).toMatch(/outline: 2px solid var\(--ring\);/);
  });

  it('sets the head’s author names at 600, and only the names (Q3-24)', () => {
    expect(
      rule(".crew-message-author [data-person-part='name'],\n.crew-message-author-lead")
    ).toMatch(/font-weight: 600;/);
  });

  it('fades a table or code block that has more to its right, and not at the end (Q3-18)', () => {
    expect(
      rule(".crew-md-code-body[data-overflow='true'],\n.crew-md-table-scroll[data-overflow='true']")
    ).toMatch(/mask-image: linear-gradient\(to right, black calc\(100% - 40px\), transparent\);/);
  });

  it('never splits a table heading or a number, and draws horizontal rules only (Q2-52)', () => {
    const cell = rule('.crew-md-cell');
    expect(cell).toMatch(/overflow-wrap: normal;/);
    expect(cell).toMatch(/padding: 11px 16px;/);
    expect(cell).not.toMatch(/border-inline-start/);
    expect(rule(".crew-md-cell[data-head='true'],\n.crew-md-cell:not(:first-child)")).toMatch(
      /white-space: nowrap;/
    );
    expect(rule('.crew-md-cell .crew-md-code-inline')).toMatch(/background-color: transparent;/);
  });

  it('gives tables tabular numbers and the element radius (T-62)', () => {
    const table = rule('.crew-md-table');
    expect(table).toMatch(/font-variant-numeric: tabular-nums;/);
    expect(table).toMatch(/border-radius: var\(--radius-element\);/);
    // A collapsed table ignores its radius.
    expect(table).toMatch(/border-collapse: separate;/);
    expect(table).toMatch(/overflow: hidden;/);
  });

  it('sets every message body on the 14/21 reading line (T-62, Q2-58)', () => {
    expect(css).toMatch(/\.crew-md,\s*\.crew-message-gutter-time \{\s*line-height: 21px;/);
  });
});
