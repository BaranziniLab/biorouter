/**
 * SD-8 — a delegated subagent's chat, opened in a browser, loads read-only
 * instead of failing to load at all.
 *
 * Measured on 2026-09-11 against a real `biorouter serve`: the tab the daemon
 * opens for a subagent it has just spawned rendered "Could not load this chat",
 * with the daemon's own refusal as the body. `POST /agent/resume` answered 403
 * for the child — `refuse_subagent_unless_user` refuses a subagent's chat to
 * anyone who cannot prove a person acted, and a keyless daemon cannot prove it
 * for anyone — while `GET /sessions/{id}` and `GET /sessions/{id}/events`
 * answered 200 for the same chat. Nothing on the daemon changes here; the store
 * stops asking the one question it knows will be refused.
 *
 * What a jsdom test can pin is what the store asks and what it paints. The
 * daemon's answers are mocked as it gave them.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MessageEvent, Session } from '../api';

const mocks = vi.hoisted(() => ({
  reply: vi.fn(),
  observeSessionEvents: vi.fn(),
  resumeAgent: vi.fn(),
  cancelTurn: vi.fn(async () => ({ data: { cancelled: true } })),
  getSession: vi.fn(),
  interrupt: vi.fn(),
  listSessions: vi.fn(async () => ({ data: { sessions: [] } })),
  updateFromSession: vi.fn(async () => ({ data: {} })),
  updateSessionUserWorkflowValues: vi.fn(async () => ({ data: {} })),
  // A browser surface states the host's provider on every reaching request
  // (`userActionHeaders` → `hostConfiguredProvider`), so this spec reads
  // `BIOROUTER_PROVIDER` the moment it sets `BROWSER_SURFACE_MARKER`. Stubbed
  // here rather than left to the network guard, which is what caught it.
  readConfig: vi.fn(async () => ({ data: 'openai' })),
}));

vi.mock('../api', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>;
  return { ...actual, ...mocks };
});

import { ChatStreamRegistry } from './chatStreamStore';
import { ChatState } from '../types/chatState';
import { BROWSER_SURFACE_MARKER } from '../utils/surface';

/**
 * The body `POST /agent/resume` answers a subagent's chat with on a keyless
 * daemon — `SUBAGENT_CONTROL_NO_KEY`, as the generated client throws it (the
 * parsed body, never an `Error`, and no status).
 */
const KEYLESS_SUBAGENT_REFUSAL = {
  message:
    'This daemon was started without a user-action key, so it cannot verify that a request ' +
    'came from the person at the keyboard, and changing, resuming, stopping or steering a ' +
    'subagent from its tab requires that proof. Nothing was changed. This control is ' +
    'unavailable on this daemon; use the desktop app.',
};

function chat(id: string, sessionType: 'sub_agent' | 'user'): Session {
  return {
    id,
    name: `Chat ${id}`,
    working_dir: '/tmp',
    session_type: sessionType,
    parent_session_id: sessionType === 'sub_agent' ? 'parent-1' : null,
    conversation: [
      {
        role: 'user',
        created: 1,
        content: [{ type: 'text', text: '## Subagent spawn context\ntask: count the files' }],
        metadata: { userVisible: true, agentVisible: false },
      },
    ],
    message_count: 1,
    total_tokens: 7,
    created_at: '',
    updated_at: '',
    extension_data: {},
    user_set_name: false,
  } as Session;
}

/** An observer feed the test writes to, and that stays open until closed. */
function controlledStream() {
  const events: MessageEvent[] = [];
  let wake: (() => void) | null = null;
  let closed = false;
  async function* stream() {
    while (!closed || events.length > 0) {
      if (events.length === 0) {
        await new Promise<void>((resolve) => {
          wake = resolve;
        });
      }
      const event = events.shift();
      if (event) yield event;
    }
  }
  return {
    stream: stream(),
    push(event: MessageEvent) {
      events.push(event);
      wake?.();
      wake = null;
    },
    close() {
      closed = true;
      wake?.();
      wake = null;
    },
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

/**
 * A fresh id per test: the transcript LRU (`utils/sessionNameSync`) is
 * module-level and keyed by session id, and a reused id would send
 * `loadSession` down the cached path, past the resume this file is about.
 */
let seq = 0;
const nextId = (label: string) => `browser-subagent-${label}-${++seq}`;

const feeds: Array<ReturnType<typeof controlledStream>> = [];

beforeEach(() => {
  for (const mock of Object.values(mocks)) mock.mockClear();
  mocks.resumeAgent.mockReset();
  mocks.getSession.mockReset();
  mocks.observeSessionEvents.mockReset();
  mocks.observeSessionEvents.mockImplementation(async () => {
    const feed = controlledStream();
    feeds.push(feed);
    return { stream: feed.stream };
  });
  // A browser has no preload bridge, so no user-action key to read.
  Object.assign(window, { electron: { showNotification: vi.fn(), logInfo: vi.fn() } });
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
  for (const feed of feeds.splice(0)) feed.close();
});

function inBrowser() {
  document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
}

describe("a subagent's chat in a browser", () => {
  it('loads from the session read when the resume is refused, and follows the child', async () => {
    inBrowser();
    const id = nextId('paint');
    mocks.resumeAgent.mockRejectedValue(KEYLESS_SUBAGENT_REFUSAL);
    mocks.getSession.mockResolvedValue({ data: chat(id, 'sub_agent') });

    const controller = new ChatStreamRegistry().getController(id);
    await controller.loadSession();

    const snapshot = controller.getSnapshot();
    expect(snapshot.sessionLoadError).toBeUndefined();
    expect(snapshot.turnError).toBeUndefined();
    expect(snapshot.session?.session_type).toBe('sub_agent');
    expect(snapshot.messages).toHaveLength(1);
    expect(snapshot.tokenState.totalTokens).toBe(7);
    expect(snapshot.chatState).toBe(ChatState.Idle);
    expect(mocks.getSession).toHaveBeenCalledWith(
      expect.objectContaining({ path: { session_id: id } })
    );

    // Followed live, as the daemon-opened tab is: the observer feed is the one
    // write-free way to watch a turn, and its connection snapshot says whether
    // the child is running — which is what puts the header's explanation up.
    await vi.waitFor(() => expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(1));
    feeds[0].push({ type: 'TurnState', active_turn_id: 'child-turn' } as unknown as MessageEvent);
    await vi.waitFor(() => expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming));

    controller.releaseOwnership();
  });

  it('never asks for the agent: no second resume, no binding, agentReady left false', async () => {
    // ⚠ `agentReady: false` is the load-bearing half. It gates the reads of
    // AGENT state, and `/agent/callable_tool_count` is `get_or_create_agent` —
    // on a miss it would mint a bare agent under the child's session id.
    inBrowser();
    const id = nextId('no-agent');
    mocks.resumeAgent.mockRejectedValue(KEYLESS_SUBAGENT_REFUSAL);
    mocks.getSession.mockResolvedValue({ data: chat(id, 'sub_agent') });

    const controller = new ChatStreamRegistry().getController(id);
    await controller.loadSession();
    // Every way back in: the tab re-mounting (the painted-session fast path),
    // and anything that waits on readiness.
    await controller.loadSession();
    await controller.whenAgentReady();

    expect(mocks.resumeAgent).toHaveBeenCalledTimes(1);
    expect(mocks.updateFromSession).not.toHaveBeenCalled();
    expect(controller.getSnapshot().agentReady).toBe(false);
    expect(controller.getSnapshot().turnError).toBeUndefined();

    controller.releaseOwnership();
  });

  it('starts no observer for a tab that closed while the read was in flight', async () => {
    inBrowser();
    const id = nextId('closed');
    mocks.resumeAgent.mockRejectedValue(KEYLESS_SUBAGENT_REFUSAL);
    const read = deferred<{ data: Session }>();
    mocks.getSession.mockReturnValue(read.promise);

    const controller = new ChatStreamRegistry().getController(id);
    const loading = controller.loadSession();
    await vi.waitFor(() => expect(mocks.getSession).toHaveBeenCalled());
    // What ChatGroupsContext does for a tab that has gone: after this, nothing
    // would ever detach an observer started for it.
    controller.releaseOwnership();
    read.resolve({ data: chat(id, 'sub_agent') });
    await loading;

    expect(mocks.observeSessionEvents).not.toHaveBeenCalled();
  });
});

describe('everything else fails exactly as it did', () => {
  it("reports the resume's refusal for a chat that is not a subagent's", async () => {
    // A browser reaching an ordinary chat it may not resume is a different
    // refusal (reach, a missing chat) and must keep its own words.
    inBrowser();
    const id = nextId('user-chat');
    mocks.resumeAgent.mockRejectedValue({ message: 'That chat is private.' });
    mocks.getSession.mockResolvedValue({ data: chat(id, 'user') });

    const controller = new ChatStreamRegistry().getController(id);
    await controller.loadSession();

    expect(controller.getSnapshot().sessionLoadError).toBe('That chat is private.');
    expect(controller.getSnapshot().session).toBeUndefined();
    expect(mocks.observeSessionEvents).not.toHaveBeenCalled();
  });

  it('reports the refusal when the subagent chat cannot be read either', async () => {
    inBrowser();
    const id = nextId('unreadable');
    mocks.resumeAgent.mockRejectedValue(KEYLESS_SUBAGENT_REFUSAL);
    mocks.getSession.mockRejectedValue({ message: 'That chat is private.' });

    const controller = new ChatStreamRegistry().getController(id);
    await controller.loadSession();

    expect(controller.getSnapshot().sessionLoadError).toBe(KEYLESS_SUBAGENT_REFUSAL.message);
    expect(mocks.observeSessionEvents).not.toHaveBeenCalled();
  });

  it('never takes the read-only path on the desktop', async () => {
    // The desktop holds the key, so a refused resume there is a real failure
    // and the reader is owed it — not a quietly painted read-only copy.
    const id = nextId('desktop');
    mocks.resumeAgent.mockRejectedValue(KEYLESS_SUBAGENT_REFUSAL);
    mocks.getSession.mockResolvedValue({ data: chat(id, 'sub_agent') });

    const controller = new ChatStreamRegistry().getController(id);
    await controller.loadSession();

    expect(controller.getSnapshot().sessionLoadError).toBe(KEYLESS_SUBAGENT_REFUSAL.message);
    expect(mocks.getSession).not.toHaveBeenCalled();
    expect(mocks.observeSessionEvents).not.toHaveBeenCalled();
  });

  it('leaves a subagent chat that resumes normally on the ordinary path', async () => {
    // Nothing about being in a browser changes a resume that succeeds (a
    // daemon that holds a key, say). Only the refusal is routed around.
    inBrowser();
    const id = nextId('resumes');
    mocks.resumeAgent.mockResolvedValue({ data: { session: chat(id, 'sub_agent') } });

    const controller = new ChatStreamRegistry().getController(id);
    await controller.loadSession();

    expect(controller.getSnapshot().sessionLoadError).toBeUndefined();
    expect(mocks.getSession).not.toHaveBeenCalled();
    expect(controller.getSnapshot().session?.id).toBe(id);
  });
});
