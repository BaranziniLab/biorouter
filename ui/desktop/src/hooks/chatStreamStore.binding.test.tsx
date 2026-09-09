/**
 * Round 3 / N1 + N3 — the renderer's copy of a chat's BINDING is kept fresh.
 *
 * The composer states the model a chat actually runs on, which is the model its
 * session row names (`restore_provider_from_session`). That is only safe while
 * this store's copy of the row is current, and until now nothing made it
 * current: `loadSession` reads the row once from `/agent/resume` and every later
 * path only patches the NAME. Two things change a binding, and each has its own
 * half here.
 *
 * **A switch** (N1). Measured: bind Codex / `gpt-6-astra`, one turn, switch the
 * app to Claude Code / `claude-fable-5-1`, reopen the chat — the composer read
 * `claude-fable-5-1` on a 1M gauge while the next turn's `token_events` recorded
 * `model_id = gpt-6-astra, provider = codex`. Preferring the row is the fix, but
 * it is only correct if a per-chat switch reaches the row immediately — otherwise
 * the composer answers the switch by showing the model just switched away from,
 * which is the regression PR #192 caught in review.
 *
 * **A turn** (N3). Measured: new chat, one turn on `versa_azure` (the row becomes
 * `versa_azure` / `private`), switch to Claude Code / `claude-opus-5`, return to
 * the tab — chip `claude-opus-5`, gauge "972.5k of 1M", no note; after
 * `location.reload()` all three were right. A turn writes fields no client can
 * compute, so this half is a re-read rather than an announcement.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { MessageEvent, Session, TokenState } from '../api';

const USER_ACTION_KEY = 'proof-of-user';

const mocks = vi.hoisted(() => ({
  reply: vi.fn(),
  observeSessionEvents: vi.fn(),
  resumeAgent: vi.fn(),
  cancelTurn: vi.fn(async () => ({ data: { cancelled: true } })),
  // ⚠ The parameter is declared even though nothing reads it here: without it
  // the mock's `.calls` type is `[][]`, and the census below — which is the
  // whole point of one of these tests — cannot index element 0.
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
import { announceSessionBinding } from '../utils/sessionBindingSync';

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

/**
 * A row bound to Codex, classified public — the state N1 measured.
 *
 * ⚠ The name is deliberately NOT a default one. `finishCurrentStream` polls
 * `GET /sessions/{id}` for an LLM-generated title whenever the name still looks
 * like a placeholder, and that poll would put unrelated calls in the same mock.
 */
function boundSession(id: string, over: Partial<Session> = {}): Session {
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
    privacy_tier: 'public',
    provider_name: 'codex',
    model_config: { model_name: 'gpt-6-astra', toolshim: false, context_limit: 400000 },
    ...over,
  } as Session;
}

/** A fresh id per test: the transcript LRU in `sessionNameSync` is module-level
 *  and keyed by id, so a reused id sends `loadSession` down the cached path. */
let sessionSeq = 0;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getSession.mockResolvedValue({ data: null });
  Object.assign(window, {
    electron: {
      getUserActionKey: vi.fn(async () => USER_ACTION_KEY),
      showNotification: vi.fn(),
      logInfo: vi.fn(),
    },
  });
});

describe('a model switch reaches the row this store holds', () => {
  it('patches the provider, the model and the window it was bound with', async () => {
    const sid = `bind-switch-${++sessionSeq}`;
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    expect(controller.getSnapshot().session?.provider_name).toBe('codex');

    announceSessionBinding({
      sessionId: sid,
      provider: 'versa_azure',
      model: 'gpt-5.5-2026-04-24',
      contextLimit: 1_050_000,
    });

    const row = controller.getSnapshot().session;
    expect(row?.provider_name).toBe('versa_azure');
    expect(row?.model_config?.model_name).toBe('gpt-5.5-2026-04-24');
    // A row naming one model beside another model's window is the lie this
    // patch exists to avoid: the gauge is sized from the pair.
    expect(row?.model_config?.context_limit).toBe(1_050_000);
    // Fields the bind did not name survive.
    expect(row?.model_config?.toolshim).toBe(false);
    expect(row?.name).toBe('A named chat');
  });

  it('clears a window it was not told, rather than keeping the previous model’s', async () => {
    const sid = `bind-nolimit-${++sessionSeq}`;
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();

    announceSessionBinding({ sessionId: sid, provider: 'ollama', model: 'qwen3.6' });

    expect(controller.getSnapshot().session?.model_config?.context_limit).toBeNull();
  });

  it('ignores an announcement for a different chat', async () => {
    const sid = `bind-other-${++sessionSeq}`;
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();

    announceSessionBinding({
      sessionId: `${sid}-someone-else`,
      provider: 'versa_azure',
      model: 'gpt-5.5-2026-04-24',
    });

    expect(controller.getSnapshot().session?.provider_name).toBe('codex');
    expect(controller.getSnapshot().session?.model_config?.model_name).toBe('gpt-6-astra');
  });

  it('does nothing before the row has loaded', () => {
    const sid = `bind-early-${++sessionSeq}`;
    const controller = new ChatStreamRegistry().getController(sid);

    announceSessionBinding({ sessionId: sid, provider: 'versa_azure', model: 'gpt-5.5' });

    expect(controller.getSnapshot().session).toBeUndefined();
  });
});

describe('a turn’s own writes are read back', () => {
  /** Every `GET /sessions/{id}` this store made, with its options. */
  function sessionReads() {
    return mocks.getSession.mock.calls.map(
      (call) =>
        call[0] as {
          path?: { session_id?: string };
          query?: { metadata_only?: boolean };
          headers?: Record<string, string>;
        }
    );
  }

  async function runOneTurn(sid: string) {
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });
    mocks.reply.mockResolvedValue({ stream: streamOf(finishFrame) });
    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await controller.handleSubmit('Reply with the single word ready.');
    return controller;
  }

  /**
   * N3, at the store. The turn raised the ratchet and rebound the provider; the
   * refresh is what makes the composer notice without a reload.
   */
  it('adopts the classification and binding the turn established', async () => {
    const sid = `bind-turn-${++sessionSeq}`;
    mocks.getSession.mockResolvedValue({
      data: boundSession(sid, {
        privacy_tier: 'private',
        privacy_reason: 'turn:versa_azure',
        provider_name: 'versa_azure',
        model_config: { model_name: 'gpt-5.5-2026-04-24', toolshim: false },
      }),
    });

    const controller = await runOneTurn(sid);

    await vi.waitFor(() => expect(controller.getSnapshot().session?.privacy_tier).toBe('private'));
    const row = controller.getSnapshot().session;
    expect(row?.provider_name).toBe('versa_azure');
    expect(row?.model_config?.model_name).toBe('gpt-5.5-2026-04-24');
    expect(row?.privacy_reason).toBe('turn:versa_azure');
  });

  it('asks for the row alone, with the proof a private chat needs to be read', async () => {
    const sid = `bind-read-${++sessionSeq}`;
    mocks.getSession.mockResolvedValue({ data: boundSession(sid, { privacy_tier: 'private' }) });

    await runOneTurn(sid);

    await vi.waitFor(() => expect(sessionReads().length).toBeGreaterThan(0));
    // ⚠ `metadata_only`, so this costs the row and not the whole transcript,
    // once per turn — and the proof, because the chat this exists to notice has
    // just become one a request without it is refused for.
    expect(sessionReads()).toEqual([
      {
        path: { session_id: sid },
        query: { metadata_only: true },
        headers: { 'X-User-Action': USER_ACTION_KEY },
        throwOnError: true,
      },
    ]);
  });

  /**
   * The response carries no conversation. Adopting it wholesale would blank the
   * transcript this store is the source of truth for — so only the four fields a
   * turn can change are merged.
   */
  it('merges four fields and leaves the transcript alone', async () => {
    const sid = `bind-merge-${++sessionSeq}`;
    mocks.getSession.mockResolvedValue({
      data: {
        id: sid,
        privacy_tier: 'private',
        provider_name: 'versa_azure',
        model_config: { model_name: 'gpt-5.5-2026-04-24', toolshim: false },
      } as Session,
    });

    const controller = await runOneTurn(sid);

    await vi.waitFor(() => expect(controller.getSnapshot().session?.privacy_tier).toBe('private'));
    expect(controller.getSnapshot().session?.name).toBe('A named chat');
    expect(controller.getSnapshot().session?.working_dir).toBe('/tmp');
    expect(controller.getSnapshot().messages.length).toBeGreaterThan(0);
  });

  /**
   * The turn has already been delivered by the time this runs. A refresh that
   * cannot complete leaves the composer exactly as stale as it was before this
   * method existed — which is not worth a toast, and must not surface as a
   * failed turn.
   */
  it('fails silently when the row cannot be re-read', async () => {
    const sid = `bind-fail-${++sessionSeq}`;
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    mocks.getSession.mockRejectedValue(new Error('daemon restarting'));

    const controller = await runOneTurn(sid);

    await vi.waitFor(() =>
      expect(warn).toHaveBeenCalledWith(
        expect.stringContaining('Failed to refresh the chat'),
        expect.any(Error)
      )
    );
    expect(controller.getSnapshot().session?.provider_name).toBe('codex');
    expect(controller.getSnapshot().turnError).toBeUndefined();
    warn.mockRestore();
  });
});
