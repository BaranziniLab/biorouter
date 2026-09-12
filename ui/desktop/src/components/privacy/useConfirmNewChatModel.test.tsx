import { act, renderHook } from '@testing-library/react';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  NEW_CHAT_MODEL_CHANGED_TITLE,
  newChatModelChangedMessage,
  useConfirmNewChatModel,
} from './useConfirmNewChatModel';

/**
 * F3 — the last look before a composer creates a new chat.
 *
 * The cross-window suite (`ModelAndProviderContext.crossWindow.test.tsx`) drives
 * this hook end to end against the real context. What is pinned here is the
 * part that must NOT refuse: a check that blocked sends on a guess would be
 * routed around, which is worse than the stale label it exists to catch.
 */

const mocks = vi.hoisted(() => ({
  state: {
    currentModel: 'gpt-5.5-2026-04-24' as string | null,
    currentProvider: 'versa_azure' as string | null,
    modelConfigStatus: 'ready' as 'ready' | 'loading',
  },
  syncAppModelSelection: vi.fn(),
  getProviders: vi.fn(),
  toastWarning: vi.fn(),
}));

vi.mock('../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    ...mocks.state,
    syncAppModelSelection: mocks.syncAppModelSelection,
  }),
}));

vi.mock('../ConfigContext', () => ({
  useConfig: () => ({ getProviders: mocks.getProviders }),
}));

vi.mock('../../toasts', () => ({ toastWarning: mocks.toastWarning }));

const confirm = async () => {
  const { result } = renderHook(() => useConfirmNewChatModel());
  let answer: boolean | undefined;
  await act(async () => {
    answer = await result.current();
  });
  return answer;
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.state.currentModel = 'gpt-5.5-2026-04-24';
  mocks.state.currentProvider = 'versa_azure';
  mocks.state.modelConfigStatus = 'ready';
  mocks.getProviders.mockResolvedValue([
    {
      name: 'claude_code',
      metadata: { name: 'claude_code', display_name: 'Claude Code' },
      resolved_tier: 'public',
    },
  ]);
});

describe('useConfirmNewChatModel', () => {
  it('proceeds when the chip already states what a new chat binds', async () => {
    mocks.syncAppModelSelection.mockResolvedValue({
      provider: 'versa_azure',
      model: 'gpt-5.5-2026-04-24',
    });
    expect(await confirm()).toBe(true);
    expect(mocks.toastWarning).not.toHaveBeenCalled();
  });

  it('refuses a known mismatch, naming the new model, its provider and its tier', async () => {
    mocks.syncAppModelSelection.mockResolvedValue({
      provider: 'claude_code',
      model: 'claude-fable-5-1',
    });
    expect(await confirm()).toBe(false);
    expect(mocks.toastWarning).toHaveBeenCalledWith({
      title: NEW_CHAT_MODEL_CHANGED_TITLE,
      msg:
        'New chats now start on claude-fable-5-1 (Claude Code, a public model), not ' +
        'gpt-5.5-2026-04-24, which this window was still showing. Your message is back in the ' +
        'composer, and the model shown below is the one it will use.',
    });
  });

  /** No evidence is not a mismatch: `createSession` reports a dead daemon. */
  it('proceeds when the daemon could not be read', async () => {
    mocks.syncAppModelSelection.mockResolvedValue(null);
    expect(await confirm()).toBe(true);
    expect(mocks.toastWarning).not.toHaveBeenCalled();
  });

  /**
   * The first frames after launch have no label on screen to be wrong about,
   * and refusing there would block the fastest send for nothing.
   */
  it('does not even look while nothing is named on screen', async () => {
    mocks.state.modelConfigStatus = 'loading';
    mocks.state.currentModel = null;
    mocks.state.currentProvider = null;
    expect(await confirm()).toBe(true);
    expect(mocks.syncAppModelSelection).not.toHaveBeenCalled();
  });

  it('still refuses when the catalog cannot name the new provider', async () => {
    mocks.getProviders.mockRejectedValue(new Error('catalog down'));
    mocks.syncAppModelSelection.mockResolvedValue({ provider: 'codex', model: 'gpt-6-astra' });
    expect(await confirm()).toBe(false);
    expect(mocks.toastWarning).toHaveBeenCalledWith(
      expect.objectContaining({
        msg: expect.stringContaining('New chats now start on gpt-6-astra (codex), not'),
      })
    );
  });
});

describe('newChatModelChangedMessage', () => {
  it('says so when no model is set for new chats any more', () => {
    expect(
      newChatModelChangedMessage(
        'gpt-5.5-2026-04-24',
        { provider: null, model: null },
        null,
        undefined
      )
    ).toBe(
      'No model is set for new chats any more — this window was still showing ' +
        'gpt-5.5-2026-04-24. Your message is back in the composer.'
    );
  });

  it('states a private tier as plainly as a public one', () => {
    expect(
      newChatModelChangedMessage(
        'claude-fable-5-1',
        { provider: 'versa_azure', model: 'gpt-5.5-2026-04-24' },
        'Versa API Azure',
        'private'
      )
    ).toContain('(Versa API Azure, a private model)');
  });

  /** An unresolved tier is not "public", and it is not said to be. */
  it('says nothing about a tier it could not resolve', () => {
    expect(
      newChatModelChangedMessage('x', { provider: 'ollama', model: 'llama' }, 'Ollama', undefined)
    ).toContain('(Ollama)');
  });
});

/**
 * ⚠ Both composers, and before `createSession`. Home (`Hub.tsx`) renders its
 * OWN `ChatInput`, not `BaseChat`'s — the app launches onto it — so a check
 * wired into one of them only is absent from half the ways a chat is started.
 * And a check placed after
 * `createSession` would run once `/agent/start` had already bound the chat.
 * jsdom mounts neither surface cheaply, so this is pinned at the source.
 */
describe('both new-chat composers look before they create', () => {
  const source = (file: string) => readFileSync(resolve(__dirname, '..', file), 'utf8');
  const CHECK = 'if (!(await confirmNewChatModel())) return false;';

  /**
   * Each composer's first side effect of a send, which the check must precede:
   * a refused send has to leave everything as it found it. Home clears the
   * pending extension overrides as it reads them; a chat marks itself as
   * creating, which blocks the composer.
   */
  it.each([
    ['Hub.tsx', 'clearExtensionOverrides();'],
    ['BaseChat.tsx', 'setIsCreatingSession(true);'],
  ])('%s checks the model before createSession and before %s', (file, firstSideEffect) => {
    const text = source(file);
    const check = text.indexOf(CHECK);
    expect(check).toBeGreaterThan(-1);
    // Exactly one check per composer — a second one would be a second policy.
    expect(text.indexOf(CHECK, check + CHECK.length)).toBe(-1);

    const sideEffect = text.indexOf(firstSideEffect, check);
    const create = text.indexOf('await createSession(', check);
    expect(sideEffect).toBeGreaterThan(check);
    expect(create).toBeGreaterThan(sideEffect);
    // And nothing of the kind slipped in ahead of it.
    expect(text.slice(Math.max(0, check - 400), check)).not.toContain(firstSideEffect);
  });
});
