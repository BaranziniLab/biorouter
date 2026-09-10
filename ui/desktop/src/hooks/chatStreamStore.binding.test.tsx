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
 *
 * **A turn's START.** The re-read above runs when the turn ENDS, so during a
 * long turn the composer still held the pre-turn classification — the ratchet
 * fires at the top of `Agent::reply`. The daemon now states the binding and the
 * post-ratchet tier in the reply stream's own first frames
 * (`PrivacyProviderPinned`), so the composer is right from the first token
 * without a request on the submit path.
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
import {
  clearSessionListCache,
  getCachedSessionList,
  subscribeSessionList,
  subscribeSessionListChanges,
  updateCachedSessionList,
} from '../utils/sessionListCache';

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

describe('a turn states what it runs on, from its first frames', () => {
  /**
   * The frame the daemon now sends on every turn. `privacy_tier` is the
   * POST-ratchet classification, so a chat this turn has just privatised says
   * `private` here while its row said `public` when the turn began.
   */
  function pinFrame(over: Partial<Record<string, unknown>> = {}): MessageEvent {
    return {
      type: 'PrivacyProviderPinned',
      provider: 'versa_azure',
      model: 'gpt-5.5-2026-04-24',
      privacy_tier: 'private',
      privacy_reason: 'turn:versa_azure',
      ...over,
    } as MessageEvent;
  }

  async function runTurn(sid: string, ...frames: MessageEvent[]) {
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });
    mocks.reply.mockResolvedValue({ stream: streamOf(...frames, finishFrame) });
    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await controller.handleSubmit('Reply with the single word ready.');
    return controller;
  }

  /**
   * The whole point, and the one field #196 measured as lagging. The post-turn
   * re-read is deliberately disarmed here (`getSession` resolves `null`, so
   * `refreshSessionBinding` returns before it patches anything) — so a `private`
   * classification on the row can ONLY have come from the turn's own frame.
   */
  it('adopts the ratcheted classification from the frame, not from a re-read', async () => {
    const sid = `turn-start-tier-${++sessionSeq}`;
    mocks.getSession.mockResolvedValue({ data: null });

    const controller = await runTurn(sid, pinFrame());

    const row = controller.getSnapshot().session;
    expect(row?.privacy_tier).toBe('private');
    expect(row?.privacy_reason).toBe('turn:versa_azure');
    // And the binding half still lands, exactly as #192 shipped it.
    expect(controller.getSnapshot().pinnedModel).toEqual({
      provider: 'versa_azure',
      model: 'gpt-5.5-2026-04-24',
    });
  });

  /**
   * A public turn's frame must not leave a provenance behind. `privacy_reason`
   * is `None` on a row that has never been raised, and the row is where the
   * chat-tab dot and the note both read from.
   */
  it('clears a provenance the chat no longer has', async () => {
    const sid = `turn-start-public-${++sessionSeq}`;
    mocks.getSession.mockResolvedValue({ data: null });
    mocks.resumeAgent.mockResolvedValue({
      data: {
        session: boundSession(sid, { privacy_tier: 'private', privacy_reason: 'turn:versa_azure' }),
      },
    });
    mocks.reply.mockResolvedValue({
      stream: streamOf(
        pinFrame({
          provider: 'codex',
          model: 'gpt-6-astra',
          privacy_tier: 'public',
          privacy_reason: null,
        }),
        finishFrame
      ),
    });
    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await controller.handleSubmit('hi');

    expect(controller.getSnapshot().session?.privacy_tier).toBe('public');
    expect(controller.getSnapshot().session?.privacy_reason).toBeNull();
  });

  /**
   * The frame arrives on EVERY turn now. A fresh session object per turn would
   * re-render the composer — chip, gauge and note included — once per turn for
   * no change at all.
   */
  it('does not churn the row when the same classification is reported again', async () => {
    const sid = `turn-start-idempotent-${++sessionSeq}`;
    mocks.getSession.mockResolvedValue({ data: null });

    const controller = await runTurn(sid, pinFrame(), pinFrame(), pinFrame());
    const first = controller.getSnapshot().session;

    mocks.reply.mockResolvedValue({ stream: streamOf(pinFrame(), finishFrame) });
    await controller.handleSubmit('again');

    expect(controller.getSnapshot().session).toBe(first);
  });

  /**
   * A frame from a daemon that predates the tier fields still binds the chip.
   * `privacy_tier` absent means "this frame knows nothing about the tier", which
   * must not be read as "public" — that would silently declassify the row this
   * client is showing.
   */
  it('leaves the classification alone when the frame carries none', async () => {
    const sid = `turn-start-legacy-${++sessionSeq}`;
    mocks.getSession.mockResolvedValue({ data: null });
    mocks.resumeAgent.mockResolvedValue({
      data: { session: boundSession(sid, { privacy_tier: 'private' }) },
    });
    mocks.reply.mockResolvedValue({
      stream: streamOf(
        {
          type: 'PrivacyProviderPinned',
          provider: 'versa_azure',
          model: 'gpt-5.5',
        } as MessageEvent,
        finishFrame
      ),
    });
    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await controller.handleSubmit('hi');

    expect(controller.getSnapshot().session?.privacy_tier).toBe('private');
    expect(controller.getSnapshot().pinnedModel?.provider).toBe('versa_azure');
  });

  /**
   * ⚠ **The frame can land BEFORE the row does, and the tier used to be lost.**
   *
   * Finding M8's second half, and the ordering is the app's own. A chat CREATED
   * by a submit is ATTACHED to — `ChatGroupsContext` observes the turn that is
   * already running — while `loadSession` is still fetching the row, and
   * `loadSession` blanks `session` for the duration of that fetch. The pin
   * survived the window because it has a home of its own; the classification's
   * only home is the row, so the updater returned `prev` and the fact was
   * dropped. Every chat-side surface then said `public` until the post-turn
   * re-read: measured in the running app on session `20260910_4` — snapshot
   * `norow` at t+559 ms with the turn already streaming, row lands `public` at
   * t+590, and does not move until t+4327, 28 ms after the turn ENDED.
   *
   * `pinnedModel` is the proof the frame really arrived: it is the half of the
   * same frame that was never lost.
   */
  it('keeps a classification the observer reported before the row loaded', async () => {
    const sid = `turn-start-norow-${++sessionSeq}`;
    // Disarm the post-turn re-read, so a `private` row can only be the frame's.
    mocks.getSession.mockResolvedValue({ data: null });
    mocks.observeSessionEvents.mockResolvedValue({ stream: streamOf(pinFrame()) });
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });

    const controller = new ChatStreamRegistry().getController(sid);
    void controller.observeSession();
    await vi.waitFor(() =>
      expect(controller.getSnapshot().pinnedModel?.model).toBe('gpt-5.5-2026-04-24')
    );
    controller.stopObserving();
    // The frame has been consumed and there is still no row — nothing invented
    // in its place.
    expect(controller.getSnapshot().session).toBeUndefined();

    await controller.loadSession();

    const row = controller.getSnapshot().session;
    expect(row?.privacy_tier).toBe('private');
    expect(row?.privacy_reason).toBe('turn:versa_azure');
  });

  /**
   * ⚠ One-shot. A stash that outlived the row it was waiting for could
   * re-assert a turn's statement over a row read later — after a
   * declassification, the one legitimate private → public move — which is the
   * unsafe direction wearing the safe one's clothes.
   */
  it('applies a pre-row classification once, and lets the next row overrule it', async () => {
    const sid = `turn-start-norow-once-${++sessionSeq}`;
    mocks.getSession.mockResolvedValue({ data: null });
    mocks.observeSessionEvents.mockResolvedValue({ stream: streamOf(pinFrame()) });
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });

    const controller = new ChatStreamRegistry().getController(sid);
    void controller.observeSession();
    await vi.waitFor(() =>
      expect(controller.getSnapshot().pinnedModel?.model).toBe('gpt-5.5-2026-04-24')
    );
    controller.stopObserving();
    await controller.loadSession();
    expect(controller.getSnapshot().session?.privacy_tier).toBe('private');

    // A row read AFTERWARDS says public — a declassification — and must stand.
    mocks.getSession.mockResolvedValue({
      data: boundSession(sid, { privacy_tier: 'public', privacy_reason: null }),
    });
    await controller.refreshSessionBinding();

    expect(controller.getSnapshot().session?.privacy_tier).toBe('public');
    expect(controller.getSnapshot().session?.privacy_reason).toBeNull();
  });

  /**
   * ⚠ The regression the widening would otherwise cause, pinned.
   *
   * `chatBinding` prefers the turn-reported pin over the row. While the frame
   * only arrived on a repaired bind that was almost never observable; now that
   * every turn reports one, a switch made AFTER a turn would be overruled by
   * that turn's pin and the chip would name the model the user just switched
   * away from — the exact regression #192 narrowed its rule to avoid.
   */
  it('lets a later switch replace the binding a turn reported', async () => {
    const sid = `turn-start-then-switch-${++sessionSeq}`;
    mocks.getSession.mockResolvedValue({ data: null });

    const controller = await runTurn(sid, pinFrame());
    expect(controller.getSnapshot().pinnedModel?.model).toBe('gpt-5.5-2026-04-24');

    announceSessionBinding({
      sessionId: sid,
      provider: 'claude_code',
      model: 'claude-opus-5',
      contextLimit: 1_000_000,
    });

    expect(controller.getSnapshot().pinnedModel).toEqual({
      provider: 'claude_code',
      model: 'claude-opus-5',
    });
    expect(controller.getSnapshot().session?.provider_name).toBe('claude_code');
  });
});

describe('a row changed elsewhere reaches this window', () => {
  /**
   * The tab dot and the sidebar read `sessionListCache`, not this store's
   * snapshot. `ChatGroupsShell` recorded that split as a KNOWN GAP — a chat that
   * ratcheted to Private during its life showed no marker on either chat-side
   * surface until something reloaded it — and said closing it needed the
   * escalation announced from the bind path. The post-turn re-read is where that
   * announcement lands.
   */
  it('patches the cached session list, so the tab dot follows the chat', async () => {
    const sid = `bind-list-${++sessionSeq}`;
    updateCachedSessionList([
      boundSession(sid),
      boundSession(`${sid}-neighbour`, { privacy_tier: 'public' }),
    ]);
    mocks.getSession.mockResolvedValue({
      data: boundSession(sid, {
        privacy_tier: 'private',
        privacy_reason: 'turn:versa_azure',
        provider_name: 'versa_azure',
        model_config: { model_name: 'gpt-5.5-2026-04-24', toolshim: false },
      }),
    });
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });
    mocks.reply.mockResolvedValue({ stream: streamOf(finishFrame) });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await controller.handleSubmit('hi');

    await vi.waitFor(() => {
      const entry = getCachedSessionList()?.find((row) => row.id === sid);
      expect(entry?.privacy_tier).toBe('private');
    });
    const entry = getCachedSessionList()?.find((row) => row.id === sid);
    expect(entry?.provider_name).toBe('versa_azure');
    expect(entry?.privacy_reason).toBe('turn:versa_azure');
    // Only this chat's entry moves.
    expect(getCachedSessionList()?.find((row) => row.id === `${sid}-neighbour`)?.privacy_tier).toBe(
      'public'
    );
  });

  /**
   * ⚠ Measured at runtime before it was a test, and it is the whole point of the
   * feed. A row refreshed from elsewhere is worthless if the composer still
   * reads the previous turn's pin: `chatBinding` prefers the pin over the row,
   * so the CLI rebound a chat to `gpt-5.2-2025-12-11`, the snapshot adopted it,
   * and the chip kept saying `gpt-5.5-2026-04-24`.
   */
  it('moves the turn-reported pin onto the row it just re-read', async () => {
    const sid = `bind-pin-follows-${++sessionSeq}`;
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });
    mocks.reply.mockResolvedValue({
      stream: streamOf(
        {
          type: 'PrivacyProviderPinned',
          provider: 'versa_azure',
          model: 'gpt-5.5-2026-04-24',
          privacy_tier: 'private',
        } as MessageEvent,
        finishFrame
      ),
    });
    // The row the daemon hands back names a DIFFERENT model — what another
    // process wrote while this window was idle. Held on a promise this test
    // resolves, so the two states can be told apart: the pin BEFORE the row is
    // read, and the pin after. Without that the refresh lands inside
    // `handleSubmit` and there is no observable "before".
    let releaseRow!: () => void;
    const rowRead = new Promise<void>((resolve) => {
      releaseRow = resolve;
    });
    mocks.getSession.mockImplementation(async () => {
      await rowRead;
      return {
        data: boundSession(sid, {
          privacy_tier: 'private',
          provider_name: 'versa_azure',
          model_config: { model_name: 'gpt-5.2-2025-12-11', toolshim: false },
        }),
      };
    });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await controller.handleSubmit('hi');
    // The turn's own report, before anything re-read the row.
    expect(controller.getSnapshot().pinnedModel?.model).toBe('gpt-5.5-2026-04-24');

    releaseRow();
    await vi.waitFor(() =>
      expect(controller.getSnapshot().pinnedModel?.model).toBe('gpt-5.2-2025-12-11')
    );
    expect(controller.getSnapshot().session?.model_config?.model_name).toBe('gpt-5.2-2025-12-11');
  });

  /**
   * ⚠ The list emit is unconditional in `sessionListCache`, and this runs after
   * EVERY turn. A refresh that found nothing new must therefore not call it at
   * all, or the sidebar, the See-all view and every tab strip wake once per turn
   * to discover that nothing moved.
   */
  it('does not touch the list when the row is already current', async () => {
    const sid = `bind-list-noop-${++sessionSeq}`;
    updateCachedSessionList([boundSession(sid)]);
    const before = getCachedSessionList();
    let emits = 0;
    const unsubscribe = subscribeSessionList(() => {
      emits += 1;
    });

    mocks.getSession.mockResolvedValue({ data: boundSession(sid) });
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });
    mocks.reply.mockResolvedValue({ stream: streamOf(finishFrame) });
    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await controller.handleSubmit('hi');
    await vi.waitFor(() => expect(mocks.getSession).toHaveBeenCalled());

    unsubscribe();
    expect(emits).toBe(0);
    expect(getCachedSessionList()).toBe(before);
  });
});

/**
 * M4 — a row read that is already out of date must not win.
 *
 * `refreshSessionBinding` is `async`, adopts the row it reads OVER the
 * turn-reported pin, and since #211 has two callers that can overlap: the end of
 * a turn, and the `/sessions/changes` nudge. Measured in the running app as a
 * composer chip going `gpt-4.1` → `gpt-5.5` → `gpt-4.1` across a single Send,
 * ending on a spelling the turn had not used.
 *
 * The daemon fixes the underlying fact (Gate B rebinds from the row, so a turn's
 * pin and its row agree). These two are the renderer's own guard, and they are
 * about ORDER rather than about which value is right.
 */
describe('a stale row read cannot overwrite a newer fact', () => {
  it('drops an answer that was requested before the pin it would overwrite', async () => {
    const sid = `bind-stale-${++sessionSeq}`;
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });

    // Held open, so the read is provably still in flight when the newer fact
    // lands. Without that there is no interleaving to test.
    let releaseRow!: () => void;
    const rowRead = new Promise<void>((resolve) => {
      releaseRow = resolve;
    });
    mocks.getSession.mockImplementation(async () => {
      await rowRead;
      // The row as it was BEFORE the switch below — the stale answer.
      return { data: boundSession(sid) };
    });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();

    const refreshed = controller.refreshSessionBinding();
    // A newer statement of the same fact, made while that read is parked.
    announceSessionBinding({
      sessionId: sid,
      provider: 'versa_azure',
      model: 'gpt-4.1-2025-04-24',
      contextLimit: 1_050_000,
    });
    releaseRow();
    await refreshed;

    expect(controller.getSnapshot().pinnedModel).toEqual({
      provider: 'versa_azure',
      model: 'gpt-4.1-2025-04-24',
    });
    expect(controller.getSnapshot().session?.provider_name).toBe('versa_azure');
    expect(controller.getSnapshot().session?.model_config?.model_name).toBe('gpt-4.1-2025-04-24');
  });

  it('drops a row that is about a different chat', async () => {
    const sid = `bind-wrongid-${++sessionSeq}`;
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });
    // Nothing downstream re-checks the id: every patch writes into THIS
    // controller's snapshot and into its entry in the shared session list. A
    // mismatched payload would silently relabel this chat with another one's
    // binding and classification.
    mocks.getSession.mockResolvedValue({
      data: boundSession(`${sid}-someone-else`, {
        privacy_tier: 'private',
        privacy_reason: 'turn:versa_azure',
        provider_name: 'versa_azure',
        model_config: { model_name: 'gpt-5.5-2026-04-24', toolshim: false },
      }),
    });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await controller.refreshSessionBinding();

    expect(controller.getSnapshot().session?.provider_name).toBe('codex');
    expect(controller.getSnapshot().session?.privacy_tier).toBe('public');
    expect(controller.getSnapshot().pinnedModel).toBeUndefined();
  });

  /**
   * The guard must not simply disable the feature. A read that STARTS after the
   * pin is the later fact and still applies — which is #211's whole purpose, and
   * the case `moves the turn-reported pin onto the row it just re-read` covers
   * through a turn. This is the same property reached directly, so a change to
   * the ordering rule cannot pass by breaking only the round-about path.
   */
  it('still adopts a row read after the pin was reported', async () => {
    const sid = `bind-fresh-${++sessionSeq}`;
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });
    mocks.getSession.mockResolvedValue({
      data: boundSession(sid, {
        provider_name: 'versa_azure',
        model_config: { model_name: 'gpt-4.1-2025-04-24', toolshim: false },
      }),
    });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    announceSessionBinding({ sessionId: sid, provider: 'codex', model: 'gpt-6-astra' });
    await controller.refreshSessionBinding();

    expect(controller.getSnapshot().pinnedModel).toEqual({
      provider: 'versa_azure',
      model: 'gpt-4.1-2025-04-24',
    });
  });
});

/**
 * Finding M8 — the chat-tab dot never turned private for a chat created in this
 * window, while the sidebar row for the SAME chat did.
 *
 * The dot read the session-list cache and nothing else. That cache cannot hold
 * a chat born in this window: `GET /sessions` INNER JOINs `messages`, so a row
 * with none is not listable, and the patch above deliberately touches only an
 * entry the cache already holds. The store knew all along — `applyTurnBinding`
 * writes the post-ratchet classification from the reply stream's first frames —
 * so the fix is to publish what the stores hold rather than to fetch anything.
 */
describe('the registry publishes what its stores say a chat is', () => {
  /**
   * Notification is batched to one animation frame (#22), and this rides that
   * same batch on purpose: the header pill, the composer and the tab dot then
   * move together rather than one of the three arriving a frame early. So the
   * wait here is a frame, not a network round trip.
   */
  const aFrame = () => new Promise((resolve) => setTimeout(resolve, 60));

  it('reports a private chat that no session list has ever carried', async () => {
    const sid = `tier-live-${++sessionSeq}`;
    mocks.resumeAgent.mockResolvedValue({
      data: { session: boundSession(sid, { privacy_tier: 'private' }) },
    });

    const registry = new ChatStreamRegistry();
    expect(registry.getSessionTiersSnapshot()).toEqual({});

    await registry.getController(sid).loadSession();
    await aFrame();

    expect(registry.getSessionTiersSnapshot()).toEqual({ [sid]: 'private' });
  });

  it('follows the ratchet a turn reports, without re-reading the row', async () => {
    const sid = `tier-ratchet-${++sessionSeq}`;
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });
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
    const controller = registry.getController(sid);
    await controller.loadSession();
    await aFrame();
    expect(registry.getSessionTiersSnapshot()).toEqual({ [sid]: 'public' });

    await controller.handleSubmit('hi');
    await vi.waitFor(async () => {
      await aFrame();
      expect(registry.getSessionTiersSnapshot()).toEqual({ [sid]: 'private' });
    });
  });

  it('emits to its subscribers only when a tier actually moved', async () => {
    const sid = `tier-quiet-${++sessionSeq}`;
    mocks.resumeAgent.mockResolvedValue({
      data: { session: boundSession(sid, { privacy_tier: 'private' }) },
    });
    mocks.getSession.mockResolvedValue({ data: boundSession(sid, { privacy_tier: 'private' }) });
    mocks.reply.mockResolvedValue({ stream: streamOf(finishFrame) });

    const registry = new ChatStreamRegistry();
    const controller = registry.getController(sid);
    await controller.loadSession();
    await aFrame();

    let emits = 0;
    const unsubscribe = registry.subscribeSessionTiers(() => {
      emits += 1;
    });
    const before = registry.getSessionTiersSnapshot();

    // A whole turn, on a chat whose tier does not move.
    await controller.handleSubmit('hi');
    await vi.waitFor(() => expect(mocks.getSession).toHaveBeenCalled());
    await aFrame();
    unsubscribe();

    expect(emits).toBe(0);
    // Identity-stable, so `useSyncExternalStore` re-renders no strip.
    expect(registry.getSessionTiersSnapshot()).toBe(before);
  });

  /**
   * ⚠ The unsafe direction, and the reason this map is a ratchet of its own.
   * A controller can momentarily hold no row — a reload, a rebind — and if the
   * published map followed it down, the strip would fall back to whatever the
   * list cache last said, which for a chat that just went private is `public`.
   * Once private, private.
   */
  it('never retracts a private it has already reported', async () => {
    const sid = `tier-noretract-${++sessionSeq}`;
    mocks.resumeAgent.mockResolvedValue({
      data: { session: boundSession(sid, { privacy_tier: 'private' }) },
    });

    const registry = new ChatStreamRegistry();
    const controller = registry.getController(sid);
    await controller.loadSession();
    await aFrame();
    expect(registry.getSessionTiersSnapshot()).toEqual({ [sid]: 'private' });

    // A binding announcement is the cheapest real path that writes the snapshot
    // without carrying a tier of its own.
    announceSessionBinding({ sessionId: sid, provider: 'ollama', model: 'qwen3.6' });
    await aFrame();

    expect(registry.getSessionTiersSnapshot()).toEqual({ [sid]: 'private' });
  });
});

/**
 * The OTHER half of M8, and deliberately not the one that fixes the dot.
 *
 * Home recents and the See-all view read the session-list cache, and a chat
 * created in this window never enters it: `notifySessionListChanged` had exactly
 * one production caller (`useDiverge`), so nothing announced a create. Announcing
 * from `createSession` cannot work either — the row has no message yet and the
 * list endpoint's INNER JOIN omits it — so the announcement is made at the first
 * moment the daemon WILL list the chat, which is after its first turn.
 */
describe('a chat born in this window tells the list it exists', () => {
  beforeEach(() => {
    clearSessionListCache();
  });

  it('announces once, when a fetched list turns out not to hold it', async () => {
    const sid = `bind-announce-${++sessionSeq}`;
    // A cache that has been fetched, and does not hold this chat.
    updateCachedSessionList([boundSession(`${sid}-neighbour`)]);
    let announcements = 0;
    const unsubscribe = subscribeSessionListChanges(() => {
      announcements += 1;
    });

    mocks.getSession.mockResolvedValue({ data: boundSession(sid, { privacy_tier: 'private' }) });
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });
    mocks.reply.mockResolvedValue({ stream: streamOf(finishFrame) });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await controller.handleSubmit('hi');
    await vi.waitFor(() => expect(announcements).toBe(1));

    // A second turn must not buy another full list fetch.
    await controller.handleSubmit('again');
    await vi.waitFor(() => expect(mocks.getSession.mock.calls.length).toBeGreaterThan(1));
    unsubscribe();
    expect(announcements).toBe(1);
  });

  it('stays quiet for a chat the list already holds', async () => {
    const sid = `bind-noannounce-${++sessionSeq}`;
    updateCachedSessionList([boundSession(sid)]);
    let announcements = 0;
    const unsubscribe = subscribeSessionListChanges(() => {
      announcements += 1;
    });

    mocks.getSession.mockResolvedValue({ data: boundSession(sid, { privacy_tier: 'private' }) });
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });
    mocks.reply.mockResolvedValue({ stream: streamOf(finishFrame) });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await controller.handleSubmit('hi');
    await vi.waitFor(() => expect(mocks.getSession).toHaveBeenCalled());
    unsubscribe();

    expect(announcements).toBe(0);
  });

  /**
   * ⚠ A null cache is "nobody has asked yet", not "the chat is missing".
   * `ChatGroupsShell` warms it on mount; answering an unfetched cache with a
   * full list fetch per turn-end would buy a page of session rows to learn one
   * string, on every chat in the window.
   */
  it('stays quiet when no list has been fetched at all', async () => {
    const sid = `bind-nolist-${++sessionSeq}`;
    let announcements = 0;
    const unsubscribe = subscribeSessionListChanges(() => {
      announcements += 1;
    });

    mocks.getSession.mockResolvedValue({ data: boundSession(sid, { privacy_tier: 'private' }) });
    mocks.resumeAgent.mockResolvedValue({ data: { session: boundSession(sid) } });
    mocks.reply.mockResolvedValue({ stream: streamOf(finishFrame) });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await controller.handleSubmit('hi');
    await vi.waitFor(() => expect(mocks.getSession).toHaveBeenCalled());
    unsubscribe();

    expect(announcements).toBe(0);
  });
});
