import type { SessionType } from '../../api';
import { isBrowserSurface } from '../../utils/surface';

/**
 * What a delegated subagent's tab says in a browser, in place of the controls
 * the daemon will refuse there.
 *
 * `docs/deployment/serve-decisions.md` **SD-7**: the daemon behind
 * `biorouter serve` holds no user-action key, so it cannot prove of any request
 * that a person made it. A subagent's chat is where that proof decides
 * everything — a message there is recorded as a person intervening, and the
 * parent is told so — and the daemon refuses every write to it from a caller
 * that cannot prove a person acted: `POST /reply`; `POST /agent/cancel`,
 * `POST /interrupt` and the two continuation routes (**SD-11**);
 * `POST /agent/stop` and the extension routes; and `POST /agent/resume`
 * itself. On a keyless daemon that is every caller, always. Those refusals are
 * the design and stay.
 *
 * **SD-8** adds that a control which can never work here says so before it is
 * touched. So the tab renders this in place of its composer
 * (`SubagentComposerSlot`) and of its Stop (`SubagentTabHeader`), the way the
 * approval card (`ToolCallConfirmation.tsx`) renders its own sentence in place
 * of Allow and Deny.
 *
 * ⚠ **These strings are for the person looking at the tab.** The daemon's own
 * refusal (`SUBAGENT_CONTROL_NO_KEY` in `routes/agent.rs`) is written for
 * whatever reads an error body, and nothing here should be copied back into
 * it.
 *
 * ⚠ **They state where the capability lives, not where to go and use it.** The
 * desktop app runs a daemon of its own; it cannot reach into a turn running
 * inside the `serve` daemon, so "stop it in the desktop app" would send a
 * person to a window that cannot stop this subagent either.
 */

/** The full reason, for the composer's slot. */
export const SUBAGENT_TAB_READ_ONLY_REASON =
  'This page is served to a browser, which has no way to prove a request came from you ' +
  'rather than from a model. Sending to, steering or stopping a delegated subagent needs ' +
  'that proof, and only the Biorouter desktop app can give it, so this chat is read-only here.';

/** The line that takes the header Stop's place, where there is room for no more. */
export const SUBAGENT_STOP_NEEDS_DESKTOP = 'Stopping needs the desktop app';

/**
 * Why a subagent's tab is read-only on this surface, or `null` where it is not
 * (the desktop, which holds the key and can prove a person acted).
 *
 * `null` rather than `''`, for the reason `hostManagedModelReason` gives: a
 * caller that spreads this into a `title` wants the attribute absent on the
 * desktop, not present and empty.
 */
export function subagentTabReadOnlyReason(): string | null {
  return isBrowserSurface() ? SUBAGENT_TAB_READ_ONLY_REASON : null;
}

/**
 * Is this a delegated subagent's chat, open on a surface where it can only be
 * read? The one predicate the chat store and the tab both ask, so the store's
 * loading and the tab's controls cannot disagree about which chats these are.
 */
export function isReadOnlySubagentChat(sessionType: SessionType | null | undefined): boolean {
  return sessionType === 'sub_agent' && isBrowserSurface();
}
