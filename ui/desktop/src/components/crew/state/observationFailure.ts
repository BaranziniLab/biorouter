import { CrewHttpError } from '../crewApi';
import { crewObservationCopy } from './copy';

/**
 * Observation error codes after which the unsent draft may no longer be sent where it was written:
 * access, privacy or the acting person changed. Any other failure keeps the draft for a retry.
 */
export const DRAFT_CLEARING_OBSERVATION_CODES: readonly string[] = [
  'channel_access_changed',
  'policy_changed',
  'scope_changed',
  'access_denied',
  'principal_revoked',
  'forbidden',
  'privacy_denied',
  'human_authority_required',
];

export interface ObservationFailureOutcome {
  /** Clear the draft, its attachments, references and context channels. */
  clearDraft: boolean;
  /** The text for the connection bar: the daemon's message, then what happened to the draft. */
  text: string;
}

/** What an observation failure does to the draft, and the sentence that says so. */
export function observationFailureOutcome(
  message: string,
  code?: string
): ObservationFailureOutcome {
  if (code && DRAFT_CLEARING_OBSERVATION_CODES.includes(code)) {
    return { clearDraft: true, text: `${message} ${crewObservationCopy.draftCleared}` };
  }
  return { clearDraft: false, text: `${message} ${crewObservationCopy.draftRetained}` };
}

/** The message of a thrown failure, or `fallback` for anything that is not an `Error`. */
export function failureMessage(failure: unknown, fallback: string): string {
  return failure instanceof Error ? failure.message : fallback;
}

/** The daemon's typed code of a thrown failure, when it has one. */
export function failureCode(failure: unknown): string | undefined {
  return failure instanceof CrewHttpError ? failure.code : undefined;
}
