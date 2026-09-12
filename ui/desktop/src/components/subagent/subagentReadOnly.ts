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
 * parent is told so — and the daemon refuses these writes to it from a caller
 * that cannot prove a person acted: `POST /reply`; `POST /agent/cancel` and
 * the two continuation routes (**SD-11**); `POST /interrupt`, which asks for the
 * proof on every daemon and so refuses here for its own reason rather than
 * SD-11's; `POST /agent/stop` and the extension routes; `POST
 * /agent/update_working_dir`; and `POST /agent/resume` itself. On a keyless
 * daemon that is every caller, always. Those refusals are the design and stay.
 *
 * ⚠ **A LIST, not "every write".** This comment claimed the latter for a day
 * and `/agent/update_working_dir` was the counterexample — it consulted only
 * the privacy reach gate, which is deliberately inert for a public session, and
 * a child's chat is normally public. It is on the list because it was gated;
 * several other session-addressing writes still reach a child's row without
 * asking (`DELETE /sessions/{id}` first among them, since it cancels the
 * child's turn), and SD-8 in `serve-decisions.md` enumerates them. Do not
 * restore the universal claim here.
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

/**
 * What this tab knows about whether its chat is a delegated subagent's.
 *
 * ⚠ **Three answers, not two, and the third is the whole point.** Every source
 * of this fact except the tab badge is an ASYNCHRONOUS read — the chat store's
 * session row, and `useSubagentSession`'s own `getSession` — and neither can
 * say "not a subagent" until it has landed. A boolean therefore reports
 * `false` for "no" and for "not yet" alike, and in a browser "not yet" lasted
 * measurably longer than the window the composer must not be offered in: with
 * a subagent running, the refused `/agent/resume` alone took 4.8 s and the
 * session read was still pending five seconds later, because those requests
 * queue behind one open event stream per observed tab against six connections
 * per origin.
 *
 * The badge does not close it either, and this is the correction the review
 * asked for: `ChatGroupsContext` holds `tabAnnotations` in ordinary React
 * state, written only from live daemon workspace frames. The layout is
 * persisted to `localStorage` per window and the annotations are NOT, so a
 * browser RELOAD restores the subagent's tab with no badge at all — and a tab
 * reached from History or a link never had one. In both cases every source read
 * `false`, and `false` mounted the composer.
 */
export type SubagentComposerKind = 'subagent' | 'other' | 'unknown';

/**
 * Which of the three this tab is in, from every source at once.
 *
 * Positive signals are ORed, because each of them is evidence and none of them
 * is required. The negative answer has exactly one source: a loaded session row
 * **for this tab's own session id**. `loadedSessionId` is compared rather than
 * assumed because `ChatGroupsShell` keys a chat by TAB id and the session
 * behind a tab is rebindable, so the store's row can still be the previous
 * chat's while a new id is loading.
 *
 * `unknown` needs something to be waiting on. A tab with no session id yet (the
 * empty tab before the first message mints one) is `other`: there is no chat
 * for a spawn to have created, and withholding its composer would leave a
 * browser unable to start one. A load that FAILED is `other` too — the tab is
 * already showing that it could not be read, and a failure is not evidence of a
 * subagent, so withholding forever would be a lockout rather than a gate.
 */
export function subagentComposerKind({
  badge,
  sessionId,
  loadedSessionId,
  loadedSessionType,
  hookSaysSubagent = false,
  loadFailed = false,
}: {
  /** The daemon's own annotation on this tab, present from mount when it exists. */
  badge?: string | null;
  /** The session this tab is showing. */
  sessionId: string;
  /** The session id the chat store has actually loaded, if any. */
  loadedSessionId?: string | null;
  /** That row's type. Only meaningful when `loadedSessionId === sessionId`. */
  loadedSessionType?: SessionType | null;
  /** `useSubagentSession`'s answer, which is positive-only (`false` until it lands). */
  hookSaysSubagent?: boolean;
  /** The store reported it could not load this chat at all. */
  loadFailed?: boolean;
}): SubagentComposerKind {
  const loaded = Boolean(sessionId) && loadedSessionId === sessionId;
  if (badge === 'subagent' || hookSaysSubagent || (loaded && loadedSessionType === 'sub_agent')) {
    return 'subagent';
  }
  if (!sessionId || loaded || loadFailed) return 'other';
  return 'unknown';
}

/** What the composer's slot puts on screen. */
export type ComposerSlotMode =
  /** The composer, untouched. */
  | 'composer'
  /** The reason there is none, in its place (SD-8). */
  | 'read-only'
  /** Nothing yet — this surface cannot offer a composer it may have to take back. */
  | 'withheld';

/**
 * The slot's one decision, as a pure function of the surface and what the tab
 * knows.
 *
 * On the desktop it is always the composer: that surface holds the user-action
 * key, so every control in there works on every chat, and there is nothing to
 * withhold or explain.
 *
 * In a browser `unknown` **withholds** rather than mounting. That direction is
 * the finding: SD-8's promise is that a control which can never work here says
 * so *before* it is touched, and a composer mounted while the answer is still
 * in flight breaks the promise for exactly the seconds in which a child is
 * running and the reader most wants to intervene. The cost is bounded and
 * almost invisible — the transcript of a browser chat does not paint until that
 * same read lands either, so what is withheld sits under an empty conversation
 * — and it is paid only on the surface that cannot prove a person acted.
 */
export function composerSlotMode(kind: SubagentComposerKind): ComposerSlotMode {
  if (!isBrowserSurface()) return 'composer';
  if (kind === 'subagent') return 'read-only';
  return kind === 'unknown' ? 'withheld' : 'composer';
}
