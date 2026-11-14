/**
 * The way to the Dictionary page from places that hold no router of their own:
 * the setup popover and **Settings > Models > Speech models** (their **Edit
 * your dictionary** links, DD-67). The app runs under a hash router, so a
 * fragment change is a route change, and the sidebar then opens its Components
 * group by the current route rule.
 */
export const DICTIONARY_ROUTE = '/dictionary';

export function openDictionaryPage(): void {
  window.location.hash = DICTIONARY_ROUTE;
}
