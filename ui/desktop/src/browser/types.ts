/**
 * The Biorouter Browser's state as the renderer sees it. Types only, so the preload bridge, the
 * main process and the renderer share one definition without the preload importing main code.
 */

/**
 * A one line note under the browser bar, with at most one action. Notes say what the browser did
 * on the person's behalf (blocked a window, refused a local address, denied a permission) and
 * offer the one way forward.
 */
export type EmbeddedBrowserNotice = {
  kind:
    | 'popup-blocked'
    | 'local-blocked'
    | 'local-reserved'
    | 'permission'
    | 'auth'
    | 'client-certificate'
    | 'external-app';
  text: string;
  action?: {
    label: string;
    /**
     * `open-tab`: open `url` in a new browser tab. `open-here`: load `url` in this tab as the
     * person's own navigation. `open-external`: hand `url` to the default browser.
     */
    kind: 'open-tab' | 'open-here' | 'open-external';
    url: string;
  };
};

export type EmbeddedBrowserDownload = {
  id: string;
  filename: string;
  receivedBytes: number;
  totalBytes: number;
  state: 'progressing' | 'completed' | 'cancelled' | 'interrupted';
};

export type EmbeddedBrowserFind = {
  activeMatchOrdinal: number;
  matches: number;
};

export type EmbeddedBrowserState = {
  url: string;
  title: string;
  managedApp?: boolean;
  /** Changes whenever the top-level document or SPA route commits. */
  sourceRevision: string;
  canGoBack: boolean;
  canGoForward: boolean;
  isLoading: boolean;
  error: string | null;
  /** The error is one the default browser may handle (a certificate, a client certificate). */
  errorOffersExternal?: boolean;
  notice?: EmbeddedBrowserNotice | null;
  download?: EmbeddedBrowserDownload | null;
  /** The page is on this computer or the local network (BB-06). */
  isLocal?: boolean;
  zoomFactor?: number;
  bookmarked?: boolean;
  find?: EmbeddedBrowserFind | null;
};

/** A browser tab a page opened (`target=_blank`, a middle click); the panel shows it as a tab. */
export type EmbeddedBrowserOpened = {
  openerViewId: string;
  viewId: string;
  url: string;
  foreground: boolean;
};

/** A browser key pressed while the page had focus, for the panel to act on. */
export type EmbeddedBrowserShortcut = {
  viewId: string;
  action: 'focus-address' | 'find' | 'close-tab';
};

export type EmbeddedBrowserSuggestion = { url: string; title: string; bookmarked: boolean };
export type EmbeddedBrowserBookmark = { url: string; title: string };
