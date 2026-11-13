/**
 * The person's half of a refused spawn (owner decision 4, 2026-11-12).
 *
 * A private chat that asks for a subagent on a public model is refused by DR-19
 * (`PrivacyRefusal::PublicChildOfPrivateParent` in
 * `crates/biorouter/src/privacy/refusal.rs`). The agent reads that refusal and is
 * told to suggest a new chat. The person is also told one more thing: if the
 * whole chat may be public, they can make it public from History and then switch
 * it to a public model, after which a subagent on a public model may start.
 *
 * Both steps are named because neither is enough alone. The refusal is about the
 * chat's model, which stays private after History removes the private marker
 * (the dialog says the chat may then "be opened by a public model"), and the
 * next turn on that model marks the chat private again. History is still the
 * first step, because a chat marked private may not be switched to a public
 * model at all.
 *
 * ⚠ **Only the person is told.** A model must never be handed a way to lower a
 * chat, so this line is not in the tool result at all. It cannot be: a refusal
 * is a failed call, and no part of a failed call's error is kept from the
 * agent (its whole text, data included, is what the stored message keeps and
 * what a coding agent's transcript carries). So the card recognises the
 * refusal itself, by the spawn tool's name and the refusal's opening words, and
 * adds the line here. `subagent_tool_person_hint_tests.rs` in
 * `crates/biorouter/src/agents/` reads this file and pins both strings below to
 * the daemon's own constants (`PUBLIC_CHILD_REFUSAL_OPENING` and
 * `MAKE_CHAT_PUBLIC_FROM_HISTORY` in `subagent_tool.rs`), and asserts the
 * refusal the model reads never carries the line. Change both together.
 *
 * ⚠ **Only for Biorouter's own spawn tool.** A tool result's text is whatever
 * the tool wrote, so a third-party tool could print the refusal's words. The
 * names below are the two spellings the daemon dispatches as a spawn
 * (`is_spawn_tool_call` in `agents/agent.rs`); a bridged coding agent's call is
 * shown under the first one.
 */

/** The spawn tool's two names: prefixed by the workspace capability, and bare. */
export const SPAWN_TOOL_NAMES: ReadonlySet<string> = new Set(['workspace__subagent', 'subagent']);

/** The words DR-19's refusal opens with. */
export const PUBLIC_CHILD_REFUSAL_OPENING =
  'This chat is private, so it cannot start a subagent on a public model';

/**
 * The line under that refusal. "Make it public from History" is how the model
 * picker already words the same way out (`SwitchModelModal.tsx`), and History's
 * own control is "Make this chat public" (`sessions/copy.ts`).
 */
export const MAKE_CHAT_PUBLIC_FROM_HISTORY =
  'If the whole chat may be public, you can make it public from History, then switch it to a public model.';

/**
 * The person-only line for a failed call, or `null` when the call is not a
 * spawn that DR-19 refused.
 */
export function spawnRefusalHint(
  toolName: string | undefined,
  errorText: string | null | undefined
): string | null {
  if (!toolName || !SPAWN_TOOL_NAMES.has(toolName)) return null;
  if (!errorText || !errorText.includes(PUBLIC_CHILD_REFUSAL_OPENING)) return null;
  return MAKE_CHAT_PUBLIC_FROM_HISTORY;
}
