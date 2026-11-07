import { trustCopy } from '../onboarding/copy';

/**
 * Fixed words for the host-key card (WI-2). The titles, the body and the first
 * two verification steps are Crew's own trust copy (`trustCopy`), so a chat
 * names a host key the way the Crew view does. Only the last step differs: the
 * Crew view's "Come back and choose Try again" names a button a chat's card
 * does not have, because the chat goes on by itself once the daemon sees the
 * key. `host` and the fingerprints are the daemon's values, sanitized by the
 * card. A chat never accepts a key: the person checks it with their IT team
 * and adds it in a terminal of their own.
 */
export const crewHostKeyStepCopy = {
  unknownTitle: trustCopy.unknownTitle,
  changedTitle: trustCopy.changedTitle,
  /** How to verify an unknown key from a chat: the Crew view's steps, then the chat's own. */
  steps: (host: string) =>
    [
      trustCopy.steps(host)[0],
      trustCopy.steps(host)[1],
      'Come back to this chat. It goes on by itself once the key is there.',
    ] as const,
  /**
   * The label over an unknown key's fingerprint. Not the Crew view's "Fingerprint the server
   * offered": the daemon may have read it with its own scan rather than from the connection
   * that was refused, and either way it is this computer that read it.
   */
  read: 'Fingerprint this computer read from the server',
  /** Under an unknown key's help: what happens once the key is added. */
  goesOn: 'Once the key is in your known-hosts file, this chat goes on by itself.',
  /** The other control on an unknown key's card: stop the chat waiting for it. */
  notNow: 'Not now',
  /** Said once the person chose Not now. */
  dismissed: 'This chat stopped waiting for the key. Ask it to try again when the key is added.',
  /** Said when the card had already ended (the key was added, or the chat moved on). */
  ended: 'This step has already ended.',
} as const;
