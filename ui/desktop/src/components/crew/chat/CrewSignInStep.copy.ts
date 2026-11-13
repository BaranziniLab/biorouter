/**
 * Fixed words for the sign-in card (the owner's D1 ruling on AO-12). `login`
 * and `server` are the daemon's values, already sanitized by the card; nothing
 * the model wrote reaches these words.
 */
export const crewSignInStepCopy = {
  title: (login: string) => `Sign in to ${login}`,
  /** What the card asks, and where the typing goes. */
  lead: (login: string, server: string) =>
    `Sign in to ${login} on ${server}. What you type goes to ssh on that server, never to this chat or the model. Type only into the Sign in window.`,
  /** Opens the Sign in window. */
  signIn: 'Sign in',
  signInLabel: (login: string, server: string) => `Sign in to ${login} on ${server}`,
  /** Ends the chat's wait with nothing signed in. */
  dismiss: 'Dismiss',
  /** Shown once the person signed in through this card. */
  signedIn: 'Signed in. The chat carries on.',
  /** Shown once the person dismissed the card. */
  dismissed: 'Dismissed. Nothing was signed in.',
  /** The daemon no longer had the card: the sign-in finished, or the chat moved on. */
  gone: 'This sign-in no longer waits for you.',
  /** The Sign in window's name for its server until the daemon names it ("Sign in to the server"). */
  windowFallback: 'the server',
  /** Shown where no Sign in window can open: a browser, or an app without one. */
  fallback:
    'Sign in is only in the desktop app. Open Crew there and choose Sign in, or run biorouter crew auth in a terminal. Never type your password into the chat.',
} as const;
