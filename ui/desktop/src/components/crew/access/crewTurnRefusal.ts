/**
 * A chat turn the daemon refused because of the chat's Crew access, read from the turn's error
 * text (final acceptance D-1).
 *
 * The daemon refuses the turn of a chat whose grant stopped, before any model request, and says why
 * in a sentence written for the person (`crates/biorouter/src/crew/mod.rs`: `GRANT_REVOKED`,
 * `GRANT_POLICY_CHANGED`, `GRANT_TIMED_OUT`, `GRANT_GONE`). A daemon from before D-1 passed the
 * workspace's own refusal on as its envelope — `Crew broker refused request: {"code":
 * "grant_expired", …}` — and the chat showed that JSON under "Model request failed". Either way the
 * chat now shows the sentence, under a title that names Crew rather than the model.
 *
 * Display only: this decides no access. The daemon refused the turn whatever is shown.
 */

/** Why the turn was refused. */
export type CrewTurnRefusalKind =
  /** The person (or another surface) revoked the chat's access. */
  | 'revoked'
  /** Crew's settings or the workspace's policy moved since the grant, so the grant ended. */
  | 'settings-changed'
  /** The grant ran out of time, or its chat's grant is no longer on this device. */
  | 'ended';

export interface CrewTurnRefusal {
  kind: CrewTurnRefusalKind;
  /** The notice's title. */
  title: string;
  /** The sentence the person reads: the daemon's own, or the same words for an old envelope. */
  message: string;
}

export const crewTurnRefusalCopy = {
  removedTitle: 'Crew access removed',
  endedTitle: 'Crew access ended',
  /** The daemon's `GRANT_POLICY_CHANGED`, word for word. */
  settingsChanged: 'Crew settings changed since access was granted. Grant access again from Crew.',
} as const;

/** An apostrophe as the daemon writes it, or as a person's keyboard might. */
const APOSTROPHE = `['’]`;

interface Rule {
  kind: CrewTurnRefusalKind;
  pattern: RegExp;
  /** The sentence to show when the text is not already one (an envelope). */
  message?: string;
}

const RULES: readonly Rule[] = [
  {
    kind: 'settings-changed',
    pattern:
      /Crew settings changed since access was granted\.(?:\s*Grant access again from Crew\.)?/,
  },
  {
    kind: 'revoked',
    pattern: new RegExp(
      `This chat${APOSTROPHE}s Crew access was removed\\.(?:\\s*Start a new chat, or grant access again from Crew\\.)?`
    ),
  },
  {
    kind: 'ended',
    pattern: new RegExp(
      `This chat${APOSTROPHE}s Crew access has ended\\.(?:\\s*Grant access again from Crew to continue\\.)?`
    ),
  },
  {
    kind: 'ended',
    pattern: new RegExp(
      `This chat${APOSTROPHE}s Crew access is no longer available\\.(?:\\s*Start a new chat, or grant access again from Crew\\.)?`
    ),
  },
  // A daemon from before D-1: the workspace's own envelope for a run it no longer honours.
  {
    kind: 'settings-changed',
    pattern:
      /"code"\s*:\s*"grant_expired"|\bgrant_expired:\s*run revoked, expired or policy changed/,
    message: crewTurnRefusalCopy.settingsChanged,
  },
];

/** The Crew refusal in `text`, or `null` when the text is about something else. */
export function crewTurnRefusal(text: string | null | undefined): CrewTurnRefusal | null {
  if (!text) return null;
  for (const rule of RULES) {
    const match = rule.pattern.exec(text);
    if (!match) continue;
    return {
      kind: rule.kind,
      title:
        rule.kind === 'revoked' ? crewTurnRefusalCopy.removedTitle : crewTurnRefusalCopy.endedTitle,
      message: rule.message ?? match[0],
    };
  }
  return null;
}

/** The Crew refusal a turn error carries, in its message or its technical details. */
export function crewTurnRefusalOf(
  error: { message?: string | null; technicalDetails?: string | null } | null | undefined
): CrewTurnRefusal | null {
  if (!error) return null;
  return crewTurnRefusal(error.message) ?? crewTurnRefusal(error.technicalDetails);
}
