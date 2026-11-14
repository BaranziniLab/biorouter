import { useSyncExternalStore } from 'react';
import {
  serveUpdateStore,
  type ServeUpdateStore,
  type ServeUpdateView,
} from '../../utils/serveUpdate';

const EMPTY: ServeUpdateView = {
  status: null,
  reconnecting: false,
  waitedSeconds: 0,
  actionError: null,
  checkRequested: false,
  dialogOpen: false,
  result: null,
};

const noop = () => () => {};

/**
 * The page's update store and its current view, or `null` off a `serve`
 * browser page (the desktop app updates through its own main process).
 */
export function useServeUpdate(store: ServeUpdateStore | null = serveUpdateStore()): {
  store: ServeUpdateStore | null;
  view: ServeUpdateView;
} {
  const view = useSyncExternalStore(
    store ? store.subscribe : noop,
    store ? store.getSnapshot : () => EMPTY,
    () => EMPTY
  );
  return { store, view };
}
