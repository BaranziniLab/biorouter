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
} as const;
