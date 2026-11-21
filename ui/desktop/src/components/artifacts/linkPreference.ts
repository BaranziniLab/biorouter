/**
 * Where a link clicked in a message opens (BB-02, Settings > Chat > Display > Open links in).
 *
 * The Biorouter Browser in the chat's panel by default, or the person's default browser. It is a
 * per viewer convenience, so it lives in this window's `localStorage`; storage that is closed or
 * cleared simply means the default.
 */
export type LinkTarget = 'biorouter' | 'default';

export const LINK_TARGET_STORAGE_KEY = 'biorouter:open-links-in';
export const DEFAULT_LINK_TARGET: LinkTarget = 'biorouter';

export function readLinkTarget(): LinkTarget {
  try {
    return localStorage.getItem(LINK_TARGET_STORAGE_KEY) === 'default' ? 'default' : 'biorouter';
  } catch {
    return DEFAULT_LINK_TARGET;
  }
}

export function writeLinkTarget(target: LinkTarget): void {
  try {
    localStorage.setItem(LINK_TARGET_STORAGE_KEY, target);
  } catch {
    // Storage closed: the default still applies.
  }
}
