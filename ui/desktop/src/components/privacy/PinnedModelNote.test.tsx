import { render, renderHook, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { PinnedModelNote } from './PinnedModelNote';
import { usePinnedModel } from './usePinnedModel';
import {
  bindingDiffersFromSelection,
  bindingLabel,
  chatBinding,
  pinnedModelNotice,
  selectionBarredByPrivacy,
} from './pinnedModel';
import type { Session } from '../../api/types.gen';

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  currentModel: 'claude-opus-5' as string | null,
  currentProvider: 'claude_code' as string | null,
}));

vi.mock('../ConfigContext', () => ({
  useConfig: () => ({ getProviders: mocks.getProviders }),
}));

vi.mock('../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    currentModel: mocks.currentModel,
    currentProvider: mocks.currentProvider,
  }),
}));

const providerRow = (name: string, display: string, tier: 'private' | 'public') => ({
  name,
  metadata: { display_name: display, tier },
  resolved_tier: tier,
});

const CATALOG = [
  providerRow('versa_azure', 'Versa API Azure', 'private'),
  providerRow('claude_code', 'Claude Code', 'public'),
];

const BINDING = { provider: 'versa_azure', model: 'gpt-5.2-2025-12-11' };

/** A row bound to `BINDING`, with the classification under test. */
const chat = (privacy_tier: 'private' | 'public'): Session =>
  ({
    id: '20260610_28',
    privacy_tier,
    provider_name: BINDING.provider,
    model_config: { model_name: BINDING.model },
  }) as Session;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getProviders.mockResolvedValue(CATALOG);
  mocks.currentModel = 'claude-opus-5';
  mocks.currentProvider = 'claude_code';
});

/**
 * F2, the verbatim repro (measured 2026-09-08): Settings → Models → Claude Code
 * / claude-opus-5, open a chat whose `privacy_tier` is `private` and send. The
 * turn answers from Versa — `restore_provider_from_session` binds the row's own
 * provider — while the composer names claude-opus-5 and sizes its gauge to
 * Claude's 1M window. Nothing on screen said so.
 */
describe('PinnedModelNote', () => {
  it('names both bindings by their display names, not their ids', async () => {
    render(<PinnedModelNote session={chat('private')} />);

    const note = await screen.findByTestId('pinned-model-note');
    await waitFor(() =>
      expect(note).toHaveTextContent(
        'This chat is marked private, so it stays on Versa API Azure / gpt-5.2-2025-12-11. ' +
          'Claude Code / claude-opus-5 is not used here.'
      )
    );
    // `versa_azure` is not what that provider is called anywhere else in the
    // app; leaking the registry id here would read as a bug in the one place
    // the user is being told something new.
    expect(note.textContent).not.toContain('versa_azure');
    expect(note.textContent).not.toContain('claude_code');
  });

  it('says nothing when the chat already runs on the selected model', () => {
    mocks.currentProvider = 'versa_azure';
    mocks.currentModel = 'gpt-5.2-2025-12-11';
    const { container } = render(<PinnedModelNote session={chat('private')} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('says nothing about a chat with no binding of its own', () => {
    const { container } = render(
      <PinnedModelNote session={{ ...chat('private'), provider_name: null } as Session} />
    );
    expect(container).toBeEmptyDOMElement();
  });

  /**
   * ⚠ The reason has to be the barrier. A PUBLIC chat bound to something other
   * than the selection was simply switched by hand, and "this chat is marked
   * private, so…" would name a cause that does not exist.
   */
  it('says nothing about a public chat that merely differs', async () => {
    const { container } = render(<PinnedModelNote session={chat('public')} />);
    // …and reads no catalog to decide it: only a private chat can ever refuse
    // the selection, so a public one costs nothing.
    await waitFor(() => expect(container).toBeEmptyDOMElement());
    expect(mocks.getProviders).not.toHaveBeenCalled();
  });

  /**
   * …and nothing when the selected model is itself private: the barrier admits
   * it, so whatever moved this chat elsewhere, privacy did not.
   */
  it('says nothing when the selected model would be admitted here', async () => {
    mocks.currentProvider = 'ollama';
    mocks.currentModel = 'qwen3.6';
    mocks.getProviders.mockResolvedValue([...CATALOG, providerRow('ollama', 'Ollama', 'private')]);
    const { container } = render(<PinnedModelNote session={chat('private')} />);
    await waitFor(() => expect(mocks.getProviders).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  /**
   * `currentProvider` / `currentModel` are `null` on the first render of every
   * chat while the config loads. A note that flashed on and off there would be
   * asserting something the component cannot yet know.
   */
  it('says nothing while the selection is still unresolved', () => {
    mocks.currentProvider = null;
    mocks.currentModel = null;
    const { container } = render(<PinnedModelNote session={chat('private')} />);
    expect(container).toBeEmptyDOMElement();
  });

  /**
   * A catalog that cannot be read yields no tier, and the sentence claims to
   * know WHY the user's choice is not in effect. Without the tier we do not.
   */
  it('says nothing when the catalog cannot be read', async () => {
    mocks.getProviders.mockRejectedValue(new Error('offline'));
    const { container } = render(<PinnedModelNote session={chat('private')} />);
    await waitFor(() => expect(mocks.getProviders).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  /**
   * …but a provider the catalog knows and cannot NAME still gets its sentence:
   * the id is true, and an invented display name would not be.
   */
  it('falls back to ids when a row carries no display name', async () => {
    mocks.getProviders.mockResolvedValue([
      { name: 'versa_azure', metadata: { tier: 'private' }, resolved_tier: 'private' },
      { name: 'claude_code', metadata: { tier: 'public' }, resolved_tier: 'public' },
    ]);
    render(<PinnedModelNote session={chat('private')} />);
    const note = await screen.findByTestId('pinned-model-note');
    expect(note).toHaveTextContent(
      'This chat is marked private, so it stays on gpt-5.2-2025-12-11. ' +
        'claude-opus-5 is not used here.'
    );
  });

  it('is a neutral note, not a warning: nothing has gone wrong', async () => {
    render(<PinnedModelNote session={chat('private')} />);
    const note = await screen.findByTestId('pinned-model-note');
    expect(note).toHaveAttribute('role', 'status');
    expect(note.className).toContain('bg-background-muted');
    expect(note.className).not.toContain('wash-warning');
    expect(note.className).not.toContain('wash-danger');
  });
});

/**
 * Round 3 / N1 — the chip and gauge state the chat's own binding for EVERY chat
 * that has one, not only for the chats the privacy barrier is holding.
 *
 * ⚠ The regression that kept #192 from shipping this, recorded because it is the
 * thing to re-check if any of these change: `changeModel` writes BOTH the
 * session row and the global default, so right after a per-chat switch the
 * daemon had them in step while the client's cached `/agent/resume` copy of the
 * row was the value from BEFORE the switch — and a composer preferring the row
 * answered the switch by showing the model the user had just switched away from.
 * It is closed at the source now (`utils/sessionBindingSync` on a switch,
 * `ChatStreamController.refreshSessionBinding` after a turn), which is what this
 * rule rests on; `hooks/chatStreamStore.binding.test.tsx` is where that half is
 * pinned.
 */
describe('what the chip and gauge are told to state', () => {
  it('states the chat’s own binding when the selection is barred from it', async () => {
    const { result } = renderHook(() => usePinnedModel(chat('private'), undefined));
    await waitFor(() => expect(result.current.effectiveModel).toEqual(BINDING));
  });

  /**
   * The residual case #192 left open. Both private, so Gate A would admit the
   * selection — but `restore_provider_from_session` binds the ROW, so the
   * selection is still not what runs here and the chip must say so. No sentence:
   * privacy is not the reason.
   */
  it('states the binding even when the selection would be admitted here', async () => {
    mocks.currentProvider = 'ollama';
    mocks.currentModel = 'qwen3.6';
    mocks.getProviders.mockResolvedValue([...CATALOG, providerRow('ollama', 'Ollama', 'private')]);
    const { result } = renderHook(() => usePinnedModel(chat('private'), undefined));
    await waitFor(() => expect(result.current.effectiveModel).toEqual(BINDING));
    expect(result.current.notice).toBeNull();
  });

  /**
   * Round 3 / N1's verbatim repro, at the hook. Bind Codex / `gpt-6-astra`, run
   * one turn (the row is written), switch the app to Claude Code /
   * `claude-fable-5-1`, reopen the chat. Measured before the fix: the composer
   * read `claude-fable-5-1` on a 1M gauge while the next turn's `token_events`
   * recorded `model_id = gpt-6-astra, provider = codex`.
   *
   * Both endpoints are public, so there is no privacy sentence to earn — and no
   * catalog read either, which is the property #192 established and this rule
   * keeps: the tier is needed only for the sentence.
   */
  it('states a public chat’s own binding, with no sentence and no catalog read', async () => {
    mocks.currentProvider = 'claude_code';
    mocks.currentModel = 'claude-fable-5-1';
    const codexChat = {
      ...chat('public'),
      provider_name: 'codex',
      model_config: { model_name: 'gpt-6-astra' },
    } as Session;

    const { result } = renderHook(() => usePinnedModel(codexChat, undefined));

    await waitFor(() =>
      expect(result.current.effectiveModel).toEqual({ provider: 'codex', model: 'gpt-6-astra' })
    );
    expect(result.current.notice).toBeNull();
    expect(mocks.getProviders).not.toHaveBeenCalled();
  });

  /**
   * The just-switched case: no stale flash. Once the row has been patched with
   * the binding the daemon accepted, it AGREES with the selection, and agreement
   * means there is nothing to override — the chip goes on rendering the
   * selection exactly as it did before the switch.
   */
  it('overrides nothing once the fresh row and the selection agree', async () => {
    mocks.currentProvider = 'versa_azure';
    mocks.currentModel = BINDING.model;
    const { result } = renderHook(() => usePinnedModel(chat('public'), undefined));
    await waitFor(() => expect(result.current.effectiveModel).toBeUndefined());
    expect(result.current.notice).toBeNull();
  });

  it('says nothing about a chat that has never named a provider', async () => {
    const { result } = renderHook(() =>
      usePinnedModel({ ...chat('public'), provider_name: null } as Session, undefined)
    );
    await waitFor(() => expect(result.current.effectiveModel).toBeUndefined());
  });

  /**
   * Round 3 / N3 — a chat whose classification was established by a turn in THIS
   * client session. The store re-reads the row after every turn, so the tier
   * arriving as `private` is all it takes: the note appears without a reload.
   */
  it('picks up a classification a turn raised, without a reload', async () => {
    const before = renderHook(({ session }) => usePinnedModel(session, undefined), {
      initialProps: { session: chat('public') },
    });
    await waitFor(() => expect(before.result.current.effectiveModel).toEqual(BINDING));
    expect(before.result.current.notice).toBeNull();

    // …and the same row after `refreshSessionBinding` adopted the ratchet.
    before.rerender({ session: chat('private') });
    await waitFor(() =>
      expect(before.result.current.notice).toBe(
        'This chat is marked private, so it stays on Versa API Azure / gpt-5.2-2025-12-11. ' +
          'Claude Code / claude-opus-5 is not used here.'
      )
    );
  });

  it('prefers what a turn reported over the row', async () => {
    const reported = { provider: 'llamacpp', model: 'gemma4' };
    const { result } = renderHook(() => usePinnedModel(chat('private'), reported));
    await waitFor(() => expect(result.current.effectiveModel).toEqual(reported));
  });

  it('states nothing while the selection is still unresolved', async () => {
    mocks.currentProvider = null;
    mocks.currentModel = null;
    const { result } = renderHook(() => usePinnedModel(chat('public'), undefined));
    await waitFor(() => expect(result.current.effectiveModel).toBeUndefined());
  });
});

describe('the pure half', () => {
  const session = (over: Partial<Session>) =>
    ({ id: 's', provider_name: 'versa_azure', ...over }) as Session;

  it('takes the binding from the session row, and a turn report outranks it', () => {
    expect(
      chatBinding(
        session({ model_config: { model_name: 'gpt-5.2-2025-12-11' } as never }),
        undefined
      )
    ).toEqual(BINDING);
    // The frame the daemon sends when the barrier repaired the binding
    // mid-turn: the row is not what ran.
    expect(
      chatBinding(session({ model_config: { model_name: 'gpt-5.2' } as never }), {
        provider: 'llamacpp',
        model: 'gemma4',
      })
    ).toEqual({ provider: 'llamacpp', model: 'gemma4' });
    // A chat that has never run has no binding of its own.
    expect(chatBinding(session({ provider_name: null }), undefined)).toBeUndefined();
    expect(chatBinding(undefined, undefined)).toBeUndefined();
  });

  it('reports a difference only when both sides are known', () => {
    const selected = { provider: 'claude_code', model: 'claude-opus-5' };
    expect(bindingDiffersFromSelection(BINDING, selected)).toBe(true);
    expect(
      bindingDiffersFromSelection({ provider: 'claude_code', model: 'claude-sonnet-5' }, selected)
    ).toBe(true);
    expect(bindingDiffersFromSelection({ ...selected }, selected)).toBe(false);
    expect(bindingDiffersFromSelection(undefined, selected)).toBe(false);
    expect(bindingDiffersFromSelection(BINDING, { provider: null, model: null })).toBe(false);
    expect(bindingDiffersFromSelection(BINDING, { provider: 'claude_code', model: null })).toBe(
      false
    );
  });

  it('blames the barrier only for a public model on a private chat', () => {
    expect(selectionBarredByPrivacy('private', 'public')).toBe(true);
    expect(selectionBarredByPrivacy('private', 'private')).toBe(false);
    expect(selectionBarredByPrivacy('public', 'public')).toBe(false);
    expect(selectionBarredByPrivacy(undefined, 'public')).toBe(false);
    expect(selectionBarredByPrivacy('private', undefined)).toBe(false);
  });

  it('drops the second clause when the selection cannot be named', () => {
    expect(pinnedModelNotice('Versa API Azure / gpt-5.5')).toBe(
      'This chat is marked private, so it stays on Versa API Azure / gpt-5.5.'
    );
  });

  it('labels a binding by provider and model, or by model alone', () => {
    expect(bindingLabel('Versa API Azure', 'gpt-5.5')).toBe('Versa API Azure / gpt-5.5');
    expect(bindingLabel(undefined, 'gpt-5.5')).toBe('gpt-5.5');
    expect(bindingLabel('   ', 'gpt-5.5')).toBe('gpt-5.5');
  });
});
