/**
 * The channel header's own strings, beside `copy.ts` (which the connection bar shares, and which
 * another area owns). Tests import these instead of retyping them.
 *
 * `slug` is the channel's slug, without the `#`.
 */
export const channelHeaderCopy = {
  /**
   * The `<h1>` trigger's accessible name, computed from its content: the visible `#name` (the `#`
   * is spoken through a visually hidden character, the glyph being decorative) and then, visually
   * hidden, ", channel menu". A heading jump reads "#general, channel menu", never
   * "general channel menu". `aria-haspopup="menu"` already says what the button opens.
   */
  menuName: (slug: string) => `#${slug}, channel menu`,
  /** The visually hidden text before the slug. */
  hash: '#',
  /** The visually hidden text after the slug. */
  menuSuffix: ', channel menu',
  /** Refresh channel's answer once the channel is verified again: shown briefly, then gone. */
  upToDate: 'Up to date',
  /** The page title while a channel is open (WCAG 2.4.2). */
  pageTitle: (slug: string, workspace: string) =>
    workspace ? `#${slug} · ${workspace} — Biorouter` : `#${slug} — Biorouter`,

  /**
   * The "Restricted" chip's tooltip. It is about which models may read the channel, and says so
   * outright, because "Restricted" beside a channel name reads as a limit on who may join
   * (Q2-65).
   */
  restrictedHint: 'Only private models can read it. It doesn’t limit who’s in the channel.',
  /**
   * The chip's accessible name after its visible "Restricted", visually hidden, so the whole name
   * is "Restricted: only private models can read it. It doesn’t limit who’s in the channel."
   */
  restrictedNameSuffix: ': only private models can read it. It doesn’t limit who’s in the channel.',

  /**
   * The channel menu's item for the Access tab. One name for one place: the tab, the workspace
   * menu and the settings tab say "Agent access" too (Q2-66).
   */
  agentAccess: 'Agent access…',

  /** What a copy item in the channel menu says, and the header announces, once it copied. */
  copied: 'Copied',
  /** …and when the clipboard refused. */
  copyFailed: 'Couldn’t copy',
} as const;
