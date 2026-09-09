import type { PinnedModelView } from '../../hooks/chatStreamStore';
import type { ProviderTier, Session, SessionClassification } from '../../api/types.gen';

/**
 * Issue #56 / F2 — what the user is told when a chat runs on a model other than
 * the one they selected.
 *
 * # The defect
 *
 * The composer's model chip and context gauge state the app's GLOBAL selection.
 * A resumed chat does not run on the global selection: `restore_provider_from_session`
 * binds the provider the SESSION ROW names (`routes/agent.rs`), and for a chat
 * marked private that is the only binding the barrier will admit — Gate A
 * refuses to attach a public provider to a private row at all.
 *
 * So: switch the app to Claude Code / claude-opus-5, watch the toast and the
 * chip change, open a private chat and send. The answer comes from Versa. The
 * chip still names claude-opus-5, and the gauge measures the usage against
 * Claude's 1M window instead of the 400k one that was really in play. Nothing on
 * screen said any of this. Measured on 2026-09-08: chip `claude-opus-5`, gauge
 * "998.6k of 1M", row `provider_name = versa_azure`, `privacy_tier = private`,
 * and after the turn `input_tokens = 27833` — a Versa turn.
 *
 * # Two statements, deliberately separated
 *
 * 1. **What runs here** — the chat's own binding, which the chip and gauge must
 *    state for EVERY chat, private or not. It says nothing about privacy and
 *    needs no permission to be true.
 * 2. **Why your choice is not in effect** — a sentence only the privacy barrier
 *    earns. {@link selectionBarredByPrivacy} is the whole test, and it is the
 *    same statement the chip's own tooltip already makes ("Private chat.
 *    Biorouter only lets a private model open it").
 *
 * Folding the two together is how an earlier draft came to say "this chat is
 * marked private, so it stays on X" about a chat that had simply been switched
 * to a different *private* provider by hand — true about the binding, wrong
 * about the reason.
 *
 * # Round 3 / N1 — statement 1 now holds for every chat
 *
 * PR #192 shipped statement 1 only where statement 2 also applied, because the
 * client's copy of the row went stale on a model switch (see
 * {@link ../privacy/usePinnedModel.usePinnedModel} for the regression that
 * forced it). Measured consequence: bind Codex / `gpt-6-astra`, run one turn,
 * switch the app to Claude Code / `claude-fable-5-1`, reopen the chat — the
 * composer read `claude-fable-5-1` on a 1M gauge while `token_events` recorded
 * `model_id = gpt-6-astra, provider = codex` for the next turn. Both endpoints
 * public, so no privacy breach; the chip, the gauge and the cost attribution
 * were all against the wrong model.
 *
 * The staleness is now fixed where it is (`utils/sessionBindingSync` on a
 * switch, `ChatStreamController.refreshSessionBinding` after a turn), so
 * statement 1 no longer has to borrow statement 2's proof.
 */

/**
 * What this chat actually runs on.
 *
 * The session row is the standing answer, and it needs no extra request: the
 * `/agent/resume` payload the chat stream already holds carries `provider_name`
 * and `model_config`, and `restore_provider_from_session` binds exactly those.
 *
 * A turn that reported its own binding (`PrivacyProviderPinned`) OUTRANKS the
 * row, and the case where they disagree is the one the frame exists for: the
 * live agent was holding something the row's classification refuses, the
 * barrier repaired it mid-turn, and the row is not what ran.
 *
 * `undefined` when the row names no provider — a chat that has never run, which
 * genuinely has no binding of its own and correctly falls back to the app's
 * selection.
 */
export function chatBinding(
  session: Session | undefined,
  reportedByTurn: PinnedModelView | undefined
): PinnedModelView | undefined {
  if (reportedByTurn) return reportedByTurn;
  const provider = session?.provider_name;
  const model = session?.model_config?.model_name;
  return provider && model ? { provider, model } : undefined;
}

/** The globally selected binding, as the composer knows it. */
export interface SelectedModel {
  provider?: string | null;
  model?: string | null;
}

/**
 * Does this chat run on something other than the app-wide selection?
 *
 * `false` — say nothing — for every uncertain case, and the uncertainty is not
 * hypothetical: `currentProvider`/`currentModel` are `null` for the first render
 * of every chat while the config loads, and the session row lands a moment
 * later. Anything that flashed on and off there would be claiming something the
 * component cannot yet know.
 */
export function bindingDiffersFromSelection(
  binding: PinnedModelView | undefined,
  selected: SelectedModel
): boolean {
  if (!binding) return false;
  const { provider, model } = selected;
  if (!provider || !model) return false;
  return binding.provider !== provider || binding.model !== model;
}

/**
 * Is the app-wide selection barred from this chat by the privacy barrier?
 *
 * Exactly `bind_allowed(public, private) == false` — a public model may not open
 * a private chat. It is not a second implementation of the gate: the gate
 * decides what may RUN and lives in the daemon, and this decides only whether a
 * sentence is warranted. Both inputs are the daemon's own answers — the row's
 * ratcheted classification and the provider catalog's instance-resolved tier.
 *
 * `false` whenever either is unresolved, and `false` when both are private:
 * a private chat on a different *private* provider is a per-chat choice, not a
 * barrier, and telling the user otherwise would name the wrong cause.
 */
export function selectionBarredByPrivacy(
  chatTier: SessionClassification | undefined,
  selectedTier: ProviderTier | undefined
): boolean {
  return chatTier === 'private' && selectedTier === 'public';
}

/**
 * The one line the user reads, in the composer.
 *
 * Written for a person, not for an agent: it names what is true of the chat,
 * what follows from it, and which choice is not in effect here — and it stops.
 * It does not tell anyone to change a setting, because the setting is not
 * wrong; it does not apologise; and it does not mention a barrier, a gate or a
 * tier, none of which the reader has to know about to understand the sentence.
 *
 * ⚠ The daemon's own refusal text (`crates/biorouter/src/privacy/refusal.rs`,
 * `routes/session_reach.rs`) is addressed to an AI agent and is pinned by
 * repo-grep tests. Nothing here may be copied back into it, and none of it
 * belongs here — the same rule `privacy/hostManagedModelCopy.ts` already
 * records for the browser-surface case.
 *
 * `selectedLabel` is omitted when the selection cannot be named, which leaves a
 * sentence that is still complete and still true.
 */
export function pinnedModelNotice(effectiveLabel: string, selectedLabel?: string): string {
  const stays = `This chat is marked private, so it stays on ${effectiveLabel}.`;
  return selectedLabel ? `${stays} ${selectedLabel} is not used here.` : stays;
}

/** `Versa API Azure / gpt-5.5-2026-04-24`, falling back to the raw id. */
export function bindingLabel(
  providerDisplayName: string | undefined | null,
  model: string
): string {
  const provider = providerDisplayName?.trim();
  return provider ? `${provider} / ${model}` : model;
}
