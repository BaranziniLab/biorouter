/**
 * The way to the Dictionary page from places that hold no router of their own:
 * the setup popover and **Settings > Models > Speech models** (their **Edit
 * your dictionary** links, DD-67). The app runs under a hash router, so a
 * fragment change is a route change, and the sidebar then opens its Components
 * group by the current route rule.
 *
 * The page has two underline tabs in its band, **Terms** and **History**
 * (DD-96). `#/dictionary?tab=history` opens the history directly, and the tab a
 * person last chose is a per viewer convenience in `localStorage`
 * ({@link DICTIONARY_TAB_STORAGE_KEY}), read and written in `try`/`catch`, so
 * the page opens on Terms wherever storage is blocked.
 */
import { DICTIONARY_LABELS } from './copy';

export const DICTIONARY_ROUTE = '/dictionary';

export type DictionaryTab = 'terms' | 'history';

/** The `localStorage` key holding the tab a person last chose. */
export const DICTIONARY_TAB_STORAGE_KEY = 'biorouter:dictionary-tab';

/** The route's query parameter naming a tab. */
export const DICTIONARY_TAB_PARAM = 'tab';

function isDictionaryTab(value: unknown): value is DictionaryTab {
  return value === 'terms' || value === 'history';
}

/** The tab a route's search names (`?tab=history`), or `null`. */
export function dictionaryTabFromSearch(search: string): DictionaryTab | null {
  try {
    const value = new URLSearchParams(search).get(DICTIONARY_TAB_PARAM);
    return isDictionaryTab(value) ? value : null;
  } catch {
    return null;
  }
}

/** The tab a person last chose here; `null` when none was stored or storage is blocked. */
export function storedDictionaryTab(): DictionaryTab | null {
  try {
    const value = window.localStorage.getItem(DICTIONARY_TAB_STORAGE_KEY);
    return isDictionaryTab(value) ? value : null;
  } catch {
    return null;
  }
}

/** Remember the tab a person chose; nothing happens where storage is blocked. */
export function rememberDictionaryTab(tab: DictionaryTab): void {
  try {
    window.localStorage.setItem(DICTIONARY_TAB_STORAGE_KEY, tab);
  } catch {
    // A per viewer convenience: the page still opens, on Terms.
  }
}

/** The tab the page opens on: the route's, else the remembered one, else Terms. */
export function initialDictionaryTab(search: string): DictionaryTab {
  return dictionaryTabFromSearch(search) ?? storedDictionaryTab() ?? 'terms';
}

/** The hash route of the page, on `tab` when one is named. */
export function dictionaryHref(tab?: DictionaryTab): string {
  return tab ? `${DICTIONARY_ROUTE}?${DICTIONARY_TAB_PARAM}=${tab}` : DICTIONARY_ROUTE;
}

/** Open the page on whichever tab it opens on. Takes no argument, so it can be a click handler. */
export function openDictionaryPage(): void {
  window.location.hash = DICTIONARY_ROUTE;
}

/** Open the page on its **History** tab. */
export function openDictationHistory(): void {
  window.location.hash = dictionaryHref('history');
}

/**
 * What each tab's page band holds (`history-design.md` 10.1), which `DictionaryView` draws from:
 * the filter's name, the ghost action, the Settings deep link the quiet icon opens, the one
 * primary, and the `⋯` items. History has no primary, as Applications, Knowledge and Chat
 * history have none (`pageBandLabels.test.ts`).
 */
export const DICTIONARY_BANDS: Readonly<
  Record<
    DictionaryTab,
    {
      filter: string;
      ghost: string | null;
      settingsSection: string;
      settingsLabel: string;
      primary: string | null;
      menu: readonly string[];
    }
  >
> = {
  terms: {
    filter: DICTIONARY_LABELS.filter,
    ghost: DICTIONARY_LABELS.export,
    settingsSection: 'dictionary',
    settingsLabel: DICTIONARY_LABELS.settingsLink,
    primary: DICTIONARY_LABELS.add,
    menu: [DICTIONARY_LABELS.deleteAll],
  },
  history: {
    filter: DICTIONARY_LABELS.history.search,
    ghost: null,
    settingsSection: 'dictation-history',
    settingsLabel: DICTIONARY_LABELS.history.settingsLink,
    primary: null,
    menu: [DICTIONARY_LABELS.history.clear],
  },
};
