/**
 * The words the shared primitives say themselves (spec 2.6, copy rules in section 1).
 *
 * Every string a primitive renders on its own lives here, so a test reads the same text the
 * component shows. A caller's own words (a row's label, an InfoTip's help) stay with the caller.
 */

/** True on macOS, where the find shortcut reads ⌘F; elsewhere it reads Ctrl+F. */
export function isMacPlatform(): boolean {
  if (typeof window !== 'undefined') {
    const electronPlatform = (window as Window & { electron?: { platform?: string } }).electron
      ?.platform;
    if (electronPlatform) return electronPlatform === 'darwin';
  }
  if (typeof navigator === 'undefined') return false;
  const platform =
    (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData?.platform ??
    navigator.platform ??
    '';
  return /mac/i.test(platform);
}

export const uiCopy = {
  /** The InfoTip trigger's accessible name. */
  infoTipName: (label: string) => `About ${label}`,
  /** The `⋯` button that opens a row's menu. */
  moreActions: 'More actions',
  /** The list filter's placeholder (History overrides it with `searchHistory`). */
  filter: 'Filter',
  /** History's placeholder for the same field. */
  searchHistory: 'Search history',
  /** The find shortcut as the person's platform spells it. */
  findShortcut: (mac: boolean = isMacPlatform()) => (mac ? '⌘F' : 'Ctrl+F'),
  /** The filter field's tooltip: its name and its shortcut. */
  filterTooltip: (label: string = 'Filter', mac: boolean = isMacPlatform()) =>
    `${label} · ${mac ? '⌘F' : 'Ctrl+F'}`,
  /** A spinner's default spoken name, when it carries one. */
  loading: 'Loading…',
} as const;
