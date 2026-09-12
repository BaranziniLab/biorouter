/**
 * The daemon renames a chat after EVERY one of its first few turns — and the
 * renderer has to notice each time, not just the first.
 *
 * `SessionManager::maybe_update_name` re-generates a chat's name on each turn
 * until it holds more than {@link AUTO_RENAME_USER_MESSAGE_LIMIT} user messages
 * (and forever while it is still on the placeholder). It runs AFTER the reply
 * stream has closed, in a spawned task, and emits no frame — so this store's
 * post-turn poll is the only thing in the app that can see it happen.
 *
 * That poll was gated on `isDefaultSessionName(session.name)`: it asked "has
 * this chat been named at all?", not "has it been RENAMED?". So it fired once,
 * for the first auto-name, and every later one was invisible to the entire
 * renderer — the tab strip, the sidebar and Home recents all kept the first
 * name until something re-read the session from the server.
 *
 * Measured in the app on 2026-09-12 (session `20260912_1`): turn 1 named it
 * "BRAVO instruction test", turn 2 renamed it in sqlite to "Penguin prompt
 * test", and 30 s later both the tab and the sidebar still read "BRAVO
 * instruction test".
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { MessageEvent, Session, TokenState } from '../api';

const USER_ACTION_KEY = 'proof-of-user';

const mocks = vi.hoisted(() => ({
  reply: vi.fn(),
  observeSessionEvents: vi.fn(),
  resumeAgent: vi.fn(),
  cancelTurn: vi.fn(async () => ({ data: { cancelled: true } })),
  getSession: vi.fn(async (_options?: unknown) => ({ data: null }) as unknown),
  interrupt: vi.fn(),
  listSessions: vi.fn(async () => ({ data: { sessions: [] } })),
  updateFromSession: vi.fn(async () => ({ data: {} })),
  updateSessionUserWorkflowValues: vi.fn(async () => ({ data: {} })),
}));

vi.mock('../api', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>;
  return { ...actual, ...mocks };
});

import { ChatStreamRegistry } from './chatStreamStore';
import {
  AUTO_RENAME_USER_MESSAGE_LIMIT,
  subscribeSessionNameChanges,
  type SessionNameChange,
} from '../utils/sessionNameSync';

const tokenState: TokenState = {
  accumulatedInputTokens: 0,
  accumulatedOutputTokens: 0,
  accumulatedTotalTokens: 0,
  inputTokens: 0,
  outputTokens: 0,
  totalTokens: 0,
};

const finishFrame = { type: 'Finish', reason: 'stop', token_state: tokenState } as MessageEvent;

async function* streamOf(...frames: MessageEvent[]) {
  for (const frame of frames) yield frame;
}

/** `user_set_name: false` — an auto-named chat, which is the only kind the
 *  daemon will rename again. */
function autoNamedSession(id: string, name: string, messages: number): Session {
  return {
    id,
    name,
    working_dir: '/tmp',
    // The count the gate reads. Role-`user` messages of the stored
    // conversation, exactly what `maybe_update_name` counts.
    conversation: Array.from({ length: messages }, (_, i) => ({
      role: 'user',
      created: i,
      content: [{ type: 'text', text: `turn ${i}` }],
      metadata: {},
    })),
    message_count: messages,
    total_tokens: 0,
    created_at: '',
    updated_at: '',
    extension_data: {},
    user_set_name: false,
    privacy_tier: 'public',
  } as unknown as Session;
}

/**
 * Every `GET /sessions/{id}` made for THIS chat.
 *
 * ⚠ Filtered by id, never counted globally. A controller from an earlier test
 * keeps walking its own 800/1200/2000… poll ladder in the background — these
 * tests run on real timers, because the poll's awaits are interleaved with them
 * — and those calls land in the same shared mock.
 */
function readsOf(sessionId: string) {
  return mocks.getSession.mock.calls
    .map(
      (call) => call[0] as { path?: { session_id?: string }; query?: { metadata_only?: boolean } }
    )
    .filter((options) => options?.path?.session_id === sessionId);
}

/** Announcements about THIS chat, for the same reason {@link readsOf} filters. */
function announcedFor(sessionId: string) {
  return announced.filter((change) => change.sessionId === sessionId);
}

/** The poll's first tick is 800 ms out. */
const PAST_FIRST_POLL = 1400;
const settle = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

let sessionSeq = 0;
let announced: SessionNameChange[] = [];
let unsubscribe: (() => void) | undefined;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getSession.mockResolvedValue({ data: null });
  announced = [];
  unsubscribe?.();
  unsubscribe = subscribeSessionNameChanges((change) => announced.push(change));
  Object.assign(window, {
    electron: {
      getUserActionKey: vi.fn(async () => USER_ACTION_KEY),
      showNotification: vi.fn(),
      logInfo: vi.fn(),
    },
  });
});

/** One turn, from a session row the daemon is still free to rename. */
async function runOneTurn(sid: string, name: string, priorUserMessages: number) {
  mocks.resumeAgent.mockResolvedValue({
    data: { session: autoNamedSession(sid, name, priorUserMessages) },
  });
  mocks.reply.mockResolvedValue({ stream: streamOf(finishFrame) });
  const controller = new ChatStreamRegistry().getController(sid);
  await controller.loadSession();
  await controller.handleSubmit('Name three species of Antarctic penguin.');
  return controller;
}

describe('the post-turn auto-rename poll', () => {
  it('announces a rename of a chat that ALREADY has a real name', async () => {
    const sid = `rename-later-${++sessionSeq}`;
    mocks.getSession.mockResolvedValue({
      data: autoNamedSession(sid, 'Penguin prompt test', 2),
    });

    const controller = await runOneTurn(sid, 'BRAVO instruction test', 1);
    await settle(PAST_FIRST_POLL);

    expect(announcedFor(sid)).toContainEqual({
      sessionId: sid,
      name: 'Penguin prompt test',
      userSetName: false,
      origin: 'llm',
    });
    // …and the store's own copy of the row moves with it, or the composer and
    // the strip would disagree inside one window.
    expect(controller.getSnapshot().session?.name).toBe('Penguin prompt test');
  });

  it('announces nothing when the daemon left the name alone', async () => {
    const sid = `rename-none-${++sessionSeq}`;
    mocks.getSession.mockResolvedValue({
      data: autoNamedSession(sid, 'BRAVO instruction test', 2),
    });

    await runOneTurn(sid, 'BRAVO instruction test', 1);
    await settle(PAST_FIRST_POLL);

    expect(announcedFor(sid)).toEqual([]);
  });

  /**
   * The gate mirrors the daemon's own rule rather than inventing one: past the
   * limit, a named chat is never renamed again, so polling for it would be up to
   * eight requests per turn asking a question whose answer cannot change.
   *
   * `getSession` is called exactly once per turn regardless — that is
   * `refreshSessionBinding`, which has always run. The poll would be the second
   * call and every one after it.
   */
  it('stops polling once the chat is past the daemon’s rename limit', async () => {
    const sid = `rename-settled-${++sessionSeq}`;
    mocks.getSession.mockResolvedValue({
      data: autoNamedSession(sid, 'A different name entirely', 9),
    });

    await runOneTurn(sid, 'BRAVO instruction test', AUTO_RENAME_USER_MESSAGE_LIMIT + 2);
    await settle(PAST_FIRST_POLL);

    expect(readsOf(sid)).toHaveLength(1);
    expect(announcedFor(sid)).toEqual([]);
  });

  /** …but a chat still stuck on the placeholder is polled however long it gets,
   *  because the daemon keeps trying to name it however long it gets. */
  it('keeps polling a chat still on the placeholder, whatever its length', async () => {
    const sid = `rename-stuck-${++sessionSeq}`;
    mocks.getSession.mockResolvedValue({ data: autoNamedSession(sid, 'Penguin prompt test', 20) });

    await runOneTurn(sid, 'New chat', AUTO_RENAME_USER_MESSAGE_LIMIT + 6);
    await settle(PAST_FIRST_POLL);

    expect(announcedFor(sid)).toContainEqual({
      sessionId: sid,
      name: 'Penguin prompt test',
      userSetName: false,
      origin: 'llm',
    });
  });

  /** The poll reads two fields; it must not drag the conversation across for
   *  them, up to eight times a turn. */
  it('reads the row metadata-only', async () => {
    const sid = `rename-metaonly-${++sessionSeq}`;
    mocks.getSession.mockResolvedValue({ data: autoNamedSession(sid, 'Penguin prompt test', 2) });

    await runOneTurn(sid, 'BRAVO instruction test', 1);
    await settle(PAST_FIRST_POLL);

    const reads = readsOf(sid);
    expect(reads.length).toBeGreaterThan(1);
    for (const read of reads) expect(read.query?.metadata_only).toBe(true);
  });
});
