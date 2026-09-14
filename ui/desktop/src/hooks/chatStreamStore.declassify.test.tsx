/**
 * A chat's classification moves in BOTH directions, and every surface that
 * draws it has to follow — defects D1 and D4 of the 2026-09-13 repair round.
 *
 * **D1 — a declassified chat's open tab kept its private icon.** Measured in
 * the running app, in one window and across two: the database, the store and
 * the sidebar all read `public`, and the tab strip's icon stayed
 * `data-privacy="private"` past twelve minutes, until a reload. The registry's
 * live map only ever rose ("mirroring the ratchet"), so the store's lowered
 * reading was thrown away and the strip's `max` held the stale one.
 *
 * **D4 — another window drew a private chat PUBLIC.** Declassify in window A
 * (the push lowers window B's History and sidebar rows, as designed), then send
 * one turn on a private model in A: the database went back to `private` /
 * `turn:versa_azure`, A's sidebar followed, and B's History row and sidebar row
 * stayed public for the whole watch — the sidebar over two minutes. Nothing
 * pushed the raise.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MessageEvent, Session, TokenState } from '../api';

const mocks = vi.hoisted(() => ({
  reply: vi.fn(),
  observeSessionEvents: vi.fn(),
  resumeAgent: vi.fn(),
  getSession: vi.fn(async (_options?: unknown) => ({ data: null }) as unknown),
  listSessions: vi.fn(async () => ({ data: { sessions: [] } })),
  updateFromSession: vi.fn(async () => ({ data: {} })),
}));

vi.mock('../api', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>;
  return { ...actual, ...mocks };
});

// The long poll is a network side effect with its own tests
// (`sessionMetaSubscription.test.ts`); here it must not be the thing that makes
// a store re-read, or a test of the row channel would pass on the poll.
vi.mock('../utils/sessionMetaSubscription', () => ({
  subscribeToSessionMeta: () => () => {},
}));

// The real channel, with the announcement observed. The positive case below
// also listens on the channel as another window would; the negative cases ask
// the spy, because "nothing arrived on a BroadcastChannel" has no barrier that
// Node orders against a different sender.
const announce = vi.hoisted(() => ({ spy: undefined as unknown as ReturnType<typeof vi.fn> }));
vi.mock('../utils/sessionRowSync', async (importOriginal) => {
  const actual = (await importOriginal()) as typeof import('../utils/sessionRowSync');
  announce.spy = vi.fn(actual.announceSessionRowChanged);
  return { ...actual, announceSessionRowChanged: announce.spy };
});

import { ChatStreamRegistry } from './chatStreamStore';

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

function row(id: string, over: Partial<Session> = {}): Session {
  return {
    id,
    name: 'A named chat',
    working_dir: '/tmp',
    conversation: [],
    message_count: 0,
    total_tokens: 0,
    created_at: '',
    updated_at: '',
    extension_data: {},
    user_set_name: true,
    privacy_tier: 'private',
    privacy_reason: 'turn:versa_azure',
    provider_name: 'versa_azure',
    model_config: { model_name: 'gpt-5.5-2026-04-24', toolshim: false, context_limit: 400000 },
    ...over,
  } as Session;
}

const publicRow = (id: string) =>
  row(id, { privacy_tier: 'public', privacy_reason: 'declassified_by_user' });

/** One animation frame: store notifications, and so the tier map, are batched to it (#22). */
const aFrame = () => new Promise((resolve) => setTimeout(resolve, 60));

/** Stands in for ANOTHER window on the row channel (Node's BroadcastChannel under jsdom). */
const ROW_CHANNEL = 'biorouter:session-row';

let seq = 0;
const sid = (label: string) => `declass-${label}-${++seq}`;

describe('the tab strip follows a declassification (D1)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getSession.mockResolvedValue({ data: null });
  });

  it('lowers a tier its store re-read, instead of holding the old private', async () => {
    const id = sid('lower');
    mocks.resumeAgent.mockResolvedValue({ data: { session: row(id) } });
    const registry = new ChatStreamRegistry();
    const controller = registry.getController(id);
    await controller.loadSession();
    await aFrame();
    expect(registry.getSessionTiersSnapshot()).toEqual({ [id]: 'private' });

    // What the change feed does ~2 s after a declassification: re-read the row.
    mocks.getSession.mockResolvedValue({ data: publicRow(id) });
    await controller.refreshSessionBinding();
    await aFrame();

    expect(controller.getSnapshot().session?.privacy_tier).toBe('public');
    expect(registry.getSessionTiersSnapshot()).toEqual({ [id]: 'public' });
  });

  it('a row read in this window makes a store holding another tier re-read at once', async () => {
    // The change feed watches at most 64 ids and the registry keeps every store
    // it ever made, so it cannot be what every store relies on.
    const id = sid('nudge');
    mocks.resumeAgent.mockResolvedValue({ data: { session: row(id) } });
    const registry = new ChatStreamRegistry();
    const stop = registry.followSessionRows();
    const otherWindow = new BroadcastChannel(ROW_CHANNEL);
    try {
      await registry.getController(id).loadSession();
      await aFrame();
      expect(registry.getSessionTiersSnapshot()).toEqual({ [id]: 'private' });

      mocks.getSession.mockResolvedValue({ data: publicRow(id) });
      // Another window declassified it and announced so.
      otherWindow.postMessage({ sessionId: id });

      await vi.waitFor(async () => {
        await aFrame();
        expect(registry.getSessionTiersSnapshot()).toEqual({ [id]: 'public' });
      });
    } finally {
      otherWindow.close();
      stop();
    }
  });
});

describe('a raise reaches every window’s list rows (D4)', () => {
  let otherWindow: BroadcastChannel;
  let heard: string[];

  beforeEach(() => {
    vi.clearAllMocks();
    announce.spy.mockClear();
    mocks.getSession.mockResolvedValue({ data: null });
    heard = [];
    otherWindow = new BroadcastChannel(ROW_CHANNEL);
    otherWindow.onmessage = (event: globalThis.MessageEvent) => {
      heard.push((event.data as { sessionId: string }).sessionId);
    };
  });

  afterEach(() => {
    otherWindow.close();
  });

  it('announces a raise its store sees', async () => {
    const id = sid('raise');
    mocks.resumeAgent.mockResolvedValue({ data: { session: publicRow(id) } });
    mocks.getSession.mockResolvedValue({ data: row(id) });
    mocks.reply.mockResolvedValue({
      stream: streamOf(
        {
          type: 'PrivacyProviderPinned',
          provider: 'versa_azure',
          model: 'gpt-5.5-2026-04-24',
          privacy_tier: 'private',
          privacy_reason: 'turn:versa_azure',
        } as unknown as MessageEvent,
        finishFrame
      ),
    });

    const registry = new ChatStreamRegistry();
    const controller = registry.getController(id);
    await controller.loadSession();
    await aFrame();
    expect(registry.getSessionTiersSnapshot()).toEqual({ [id]: 'public' });

    await controller.handleSubmit('hi');

    await vi.waitFor(() => expect(heard).toContain(id));
    expect(announce.spy).toHaveBeenCalledWith(id);
  });

  it('announces nothing for a store’s first reading of a chat', async () => {
    const id = sid('first');
    mocks.resumeAgent.mockResolvedValue({ data: { session: row(id) } });

    const registry = new ChatStreamRegistry();
    await registry.getController(id).loadSession();
    await aFrame();
    expect(registry.getSessionTiersSnapshot()).toEqual({ [id]: 'private' });

    await aFrame();
    expect(announce.spy).not.toHaveBeenCalledWith(id);
  });

  it('does not announce again a change this window was already handed', async () => {
    // Window B, told of A's declassification: its lists read the row, and its
    // own store then catches up. Announcing that would make every window read
    // the row a second time for nothing.
    const id = sid('echo');
    mocks.resumeAgent.mockResolvedValue({ data: { session: row(id) } });
    const registry = new ChatStreamRegistry();
    const stop = registry.followSessionRows();
    try {
      await registry.getController(id).loadSession();
      await aFrame();

      mocks.getSession.mockResolvedValue({ data: publicRow(id) });
      otherWindow.postMessage({ sessionId: id });
      await vi.waitFor(async () => {
        await aFrame();
        expect(registry.getSessionTiersSnapshot()).toEqual({ [id]: 'public' });
      });

      await aFrame();
      expect(announce.spy).not.toHaveBeenCalledWith(id);
    } finally {
      stop();
    }
  });
});
