/**
 * The layout's own strings (ui-redesign-spec, "Main-area states outside a channel", the composer's
 * note slot and "Copy deck"). Every area keeps its strings in its own `copy.ts`; these are the few
 * the composition itself shows. Tests import them instead of retyping them.
 *
 * American English, sentence case, typographic apostrophes. A string that names a person takes it
 * already formatted by `personLabel`, so nothing here formats a person or prints an ID.
 */
import { checklistCopy, hostCopy } from '../onboarding/copy';

export const layoutCopy = {
  /** The main area's `<h1>` outside a channel, when no workspace is selected yet. */
  crew: 'Crew',
  /** Read while the first list of saved workspaces loads (the skeleton is decoration). */
  loading: 'Loading Crew…',
  /**
   * `checking`: a saved workspace whose first verified view has not arrived. The status row says
   * "Checking connection" (pinned); this names the placeholder without repeating it.
   */
  loadingChannels: 'Loading your channels…',

  /** `updates-paused`: under the connection bar, which holds the one Retry. */
  updatesPaused: 'Messages will show here again once live updates are back.',
  /** `updates-paused` with no saved connection loaded at all: nothing in the bar can retry. */
  tryAgain: 'Try again',

  /** A toast for the host: `person` is `personLabel(…, 'inline')`, e.g. "Bob Lee (@bob)". */
  joined: (person: string, workspace: string) => `${person} joined ${workspace}`,
  /**
   * A toast when someone adds you to a channel while you are elsewhere (Q2-63). `person` is
   * `@username` (or `personLabel(…, 'inline')` when there is none); `channel` is `#name`.
   */
  channelAdded: (person: string, channel: string) => `${person} added you to ${channel}`,

  ownership: {
    /** `owner` is `personLabel(…, 'authority')`; `channel` is `#name`. */
    offeredBy: (owner: string, channel: string) => `${owner} offered you ownership of ${channel}.`,
    offered: (channel: string) => `You’ve been offered ownership of ${channel}.`,
    accept: 'Accept ownership',
  },

  /**
   * The host's institution note: the one irreversible step a Private workspace still needs.
   * Worded as the Host flow's label step, and acting through the same confirmation.
   */
  institution: {
    title: hostCopy.labelTitle,
    body: hostCopy.labelBody,
    set: checklistCopy.setInstitution,
    later: hostCopy.labelLater,
  },
} as const;
