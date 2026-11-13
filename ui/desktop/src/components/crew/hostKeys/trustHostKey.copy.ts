/**
 * Fixed words for "Trust this key" (the owner's D2 ruling on AO-12), the same in Crew's trust
 * panes and a chat's host-key card. The daemon writes a key only when no key is known for the
 * server and the key it reads again has the fingerprint shown; a changed key is never offered.
 * Nothing here comes from a model: the host and the fingerprint are the daemon's values,
 * sanitized by the caller.
 */
export const trustHostKeyCopy = {
  /** Beside the button, always. */
  warning: 'Only trust it if IT or the server’s admin gave you this fingerprint.',
  desktopRequired: 'Open the Biorouter desktop app to read or trust this key.',
  trust: 'Trust this key',
  trusting: 'Trusting…',
  /** Once the daemon added the key. */
  trusted: 'Trusted. The key is in your known hosts file.',
  /** After "Trusted." where the person starts the step again (a setup's Try again). */
  tryAgainNote: 'Choose Try again.',
  /** When the known hosts file already held exactly this key. */
  alreadyTrusted: 'This computer already trusts this key.',
  /** A refusal the daemon gave no sentence for. */
  refused: 'Crew couldn’t trust this key. Add it in a terminal of your own.',
  /** While the daemon reads a fingerprint for a pane whose refusal printed none. */
  reading: 'Reading the server’s fingerprint…',
  /** When the daemon could read no fingerprint, and gave no sentence. */
  unreadable:
    'This computer couldn’t read the server’s fingerprint. Add the key in a terminal of your own.',
} as const;
