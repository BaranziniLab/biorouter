/**
 * Account ownership's strings (docs/crew/design/account-ownership.md, "Desktop"): adding this
 * computer again, the account's devices, the version and new-device notices, setups a chat
 * started, and the chats that read a workspace as the person. American English, sentence case,
 * typographic apostrophes, no dashes. A string a test asserts is read from here.
 */

export const rejoinCopy = {
  /**
   * `crew_device_key_missing` (AO-9): the daemon's sentence, for when the bar has no code of its
   * own to show.
   */
  keyMissing: (workspace: string) =>
    `This computer no longer has its key for ${workspace}. Choose Add this computer again to make a new one.`,
  /** `crew_device_not_recognized` (AO-9). */
  notRecognized: (workspace: string) =>
    `${workspace} no longer recognizes this computer. Choose Add this computer again while you are still a member.`,
  /** The person removed this computer from Keys (AO-5). */
  removedHere: (workspace: string) =>
    `You removed this computer from ${workspace}. Choose Add this computer again to use it here.`,
  action: 'Add this computer again',
  pending: 'Adding this computer…',
  /** Once the workspace admitted the new key. Chats need access again: the policy epoch moved. */
  done: (workspace: string) =>
    `This computer is back in ${workspace}. Chats that had Crew access through it need access again.`,
  /** The cap of 32 made room by removing the oldest device (AO-2). */
  evicted: (fingerprint: string) =>
    `To make room, the workspace removed your oldest device, ${fingerprint}.`,
  failed: 'Couldn’t add this computer again.',
} as const;

export const versionCopy = {
  /** The workspace's server runs an older Crew than the one this build carries (AO-4, AO-9). */
  older: (workspace: string) => `${workspace}’s server runs an older Crew.`,
  /** To the host: the action. */
  update: 'Update Crew',
  /** To everyone else: whom to ask. `host` is `@username`, or null when the view does not say. */
  askHost: (host: string | null) =>
    host ? `Ask ${host} to update Crew.` : 'Ask your host to update Crew.',
  dismiss: 'Dismiss',
} as const;

export const newDevicesCopy = {
  /** AO-10: from the daemon's `new_devices`, until acknowledged. `when` is a calendar date. */
  one: (when: string | null) =>
    when
      ? `A new device was added to your account on ${when}.`
      : 'A new device was added to your account.',
  many: (count: number) => `${count} new devices were added to your account.`,
  /** How it was added, after the sentence. */
  viaAccount: 'It signed in to the server as you.',
  review: 'Review',
  reviewName: 'Review devices',
  acknowledge: 'Acknowledge',
  acknowledgeFailed: 'Couldn’t mark the new devices as seen. Try again.',
} as const;

export const deviceCopy = {
  remove: 'Remove',
  removeLabel: (fingerprint: string) => `Remove device ${fingerprint}`,
  removeThisLabel: 'Remove this computer',
  /** AO-5: revoking is the second step; SSH access is the first. */
  sshWarning:
    'A lost computer can still sign in as you while it has an SSH key or password for your account. First remove its key from ~/.ssh/authorized_keys on the server, or change your password. Then remove it here.',
  confirm: (fingerprint: string) => `Remove ${fingerprint} from your account?`,
  confirmThis: 'Remove this computer from your account?',
  /** Removing this computer: what follows. */
  thisFollows:
    'This computer stops reaching the workspace until you choose Add this computer again.',
  confirmAction: 'Remove device',
  keep: 'Keep',
  removed: (fingerprint: string) => `Removed ${fingerprint}.`,
  removedThis: 'Removed this computer. Choose Add this computer again to use the workspace here.',
  failed: 'Couldn’t remove the device.',
} as const;

export const setupJobsCopy = {
  /** A setup a chat or a terminal started (AO-8, AO-12), still running. */
  running: (server: string) => `Crew is being set up on ${server}.`,
  /** The same, waiting for the person: only they can sign in. */
  needsSignIn: (server: string) => `A setup on ${server} is waiting for you to sign in.`,
  signIn: 'Sign in',
  signInLabel: (server: string) => `Sign in to ${server}`,
  theServer: 'a server',
} as const;

export const readsCopy = {
  heading: 'Reads as you',
  /** AO-12's read consent, one per workspace. */
  reads: (workspace: string) => `Reads ${workspace} as you`,
  since: (date: string) => `Since ${date}`,
  revoke: 'Revoke',
  revokeLabel: (workspace: string) => `Stop this chat reading ${workspace} as you`,
  revoked: (workspace: string) => `This chat no longer reads ${workspace} as you.`,
  failed: 'Couldn’t stop it. Try again.',
  /** The consent was already gone (404 `crew_not_found`). */
  gone: 'This chat had no permission left to take back.',
} as const;
