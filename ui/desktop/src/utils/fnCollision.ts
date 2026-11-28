/**
 * The apps Biorouter knows to act on the fn key, so its dictation key can step aside for them
 * (DD-101). The `biorouter-fnkey` helper reports the bundle ids of the apps other than Apple's that
 * hold an enabled event tap on modifier keys (its `listeners` line, read from `CGGetEventTapList`,
 * which asks for no permission). That list alone is no collision: Siri watches modifier keys on
 * every Mac, and hotkey tools commonly do too. A collision is an app on this reviewed list that is
 * in that report.
 *
 * Only this table names an app anywhere in the interface or a log, so no text another program
 * controls ever reaches either. An app is added only with a recorded measurement that it acts on
 * fn by default; an app that is not listed is never named and never switches Automatic.
 */

/** An app that acts on fn by default: its bundle id, and the name Biorouter shows for it. */
export interface KnownFnApp {
  readonly bundle: string;
  readonly name: string;
}

/** Apps that act on fn by default. An app is added only with a recorded measurement that it does. */
export const KNOWN_FN_APPS: readonly KnownFnApp[] = Object.freeze([
  // fn is its push to talk (measured 2026-11-26 on the owner's Mac, Wispr Flow 1.6.1631).
  Object.freeze({ bundle: 'com.electron.wispr-flow', name: 'Wispr Flow' }),
]);

/** The listed apps among `bundles`, in table order. Bundle ids match exactly, case included. */
export function listedFnApps(
  bundles: readonly string[],
  table: readonly KnownFnApp[] = KNOWN_FN_APPS
): KnownFnApp[] {
  return table.filter((app) => bundles.includes(app.bundle));
}

/**
 * The app that collides with fn: the first entry of the table whose bundle id is among `bundles`,
 * or null. The answer is the table's own entry, never anything another program wrote.
 */
export function collidingFnApp(
  bundles: readonly string[],
  table: readonly KnownFnApp[] = KNOWN_FN_APPS
): KnownFnApp | null {
  return listedFnApps(bundles, table)[0] ?? null;
}
