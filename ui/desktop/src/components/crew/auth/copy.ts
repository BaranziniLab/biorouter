/**
 * The sign-in strings (ui-redesign-spec, copy deck "Connection problems and sign in"). A string a
 * test asserts is read from here.
 */
export const signInCopy = {
  title: (host: string) => `Sign in to ${host}`,
  lead: 'Type your password or verification code in the box below. Nothing you type is saved.',
  /** The visible text of the component's close control. */
  close: 'Close',
  /** Pinned: the close control's accessible name. */
  closeName: 'Close authentication connection',
  /** Pinned fragment: "SSH authentication ended (exit {code})". */
  ended: (code: number | string) =>
    `SSH authentication ended (exit ${code}). Choose Reconnect to check the connection.`,
  inputLost: 'Your input couldn’t reach the server. Close this sign-in and try again.',
  needsDesktop: 'Signing in needs the Biorouter desktop app.',
  terminalName: 'SSH authentication',
  help: 'Trouble signing in?',
  sameCredentials: 'Use the same username and password you use for this server.',
  jumpHost: 'If your IT team gave you a jump host, add it in Connection settings.',
  knownHosts: 'Crew checks servers against this file:',
  knownHostsLabel: 'known-hosts path',
} as const;
