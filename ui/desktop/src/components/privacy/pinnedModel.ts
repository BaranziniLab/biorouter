import type { PinnedModelView } from '../../hooks/chatStreamStore';

/**
 * Issue #56 Gate B, repair arm — what the user is told when a private chat runs
 * on a model other than the one they selected.
 *
 * # The defect this answers
 *
 * A private chat may never reach a public model. When the app's globally
 * selected model is public and the user sends into such a chat, the agent
 * re-binds to the provider the SESSION ROW names and answers from there. That
 * repair is right: refusing the turn would strand the user, and asking them to
 * downgrade the chat would trade the guarantee away to fix a display problem.
 *
 * What was wrong is that nothing said so. The user had switched models in
 * Settings, watched a toast confirm it, watched the composer's chip change —
 * and then got an answer from a different model, with the context gauge sized
 * to the wrong window (measured: chip `claude-opus-5`, gauge "969.9k of 1M",
 * while the row still read `provider_name = versa_azure` and its token counts
 * matched Versa).
 *
 * # Why the daemon does not decide this
 *
 * The frame the daemon sends says only "this turn ran on `provider`/`model`",
 * because the agent's own view of what it displaced is incomplete — an
 * LRU-rehydrated agent was holding nothing at all, and a repair from "nothing"
 * to the row's own binding is not news to anybody. The client knows exactly
 * what it is showing the user, so the client is the only side that can tell a
 * silent correction from a contradiction. That comparison is
 * {@link pinContradictsSelection}, and it is a pure function so it can be
 * tested without a daemon, a socket or a render.
 */

/** The globally selected binding, as the composer knows it. */
export interface SelectedModel {
  provider?: string | null;
  model?: string | null;
}

/**
 * Does the pin contradict what the user picked?
 *
 * `false` — say nothing — for every uncertain case, and the uncertainty is not
 * hypothetical: `currentProvider`/`currentModel` are `null` for the first
 * render of every chat while the config loads. A note that flashed on and off
 * there would be worse than no note, and it would be claiming something the
 * component cannot yet know.
 */
export function pinContradictsSelection(
  pinned: PinnedModelView | undefined,
  selected: SelectedModel
): boolean {
  if (!pinned) return false;
  const { provider, model } = selected;
  if (!provider || !model) return false;
  return pinned.provider !== provider || pinned.model !== model;
}

/**
 * The one line the user reads, in the composer.
 *
 * Written for a person, not for an agent: it names what is true of the chat,
 * what follows from it, and which choice is not in effect here — and it stops.
 * It does not tell anyone to change a setting, because the setting is not
 * wrong; it does not apologise; and it does not describe a barrier, a gate or a
 * tier, none of which is a thing the reader has to know about to understand the
 * sentence.
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
