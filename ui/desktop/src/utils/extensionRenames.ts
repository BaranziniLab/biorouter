/**
 * The extension ids Biorouter has renamed, `from` -> `to`: the desktop's mirror
 * of `crates/biorouter/src/config/extension_renames.json`.
 *
 * ⚠ **A mirror, pinned by `extensionRenames.test.ts`, which reads the JSON.**
 * The table is the JSON; `crates/biorouter/src/config/extension_renames.rs`
 * reads it compiled in, and `landing/scripts/build-registry.mjs` reads it from
 * disk. The desktop keeps its own copy only because main-process and renderer
 * code does not import files from outside `ui/desktop`. Edit the two together.
 *
 * What the desktop uses it for: installing a bundle whose manifest still says
 * an old id (`gptragent.brxt` installs as `gptr`), knowing that a marketplace
 * row with the old id is installed under the new one, and the extension
 * updater's expected bundle names. It decides no privacy: both names of a
 * renamed private extension are in the private set
 * (`components/settings/extensions/extensionPrivacy.ts`).
 *
 * A row is never removed: old transcripts and workflows keep naming the old id.
 */

export interface ExtensionRename {
  /** The id the extension had before. Already a `nameToKey` key. */
  readonly from: string;
  /** The id it has now. */
  readonly to: string;
}

export const EXTENSION_RENAMES: readonly ExtensionRename[] = [{ from: 'gptragent', to: 'gptr' }];

/** The same reduction as Rust's `name_to_key`: whitespace stripped, lowercased. */
function toKey(name: string): string {
  return name.replace(/\s+/g, '').toLowerCase();
}

/** The id an extension known as `name` has now: `gptragent` -> `gptr`. */
export function currentExtensionKey(name: string): string {
  const key = toKey(name);
  return EXTENSION_RENAMES.find((rename) => rename.from === key)?.to ?? key;
}

/** Every id the extension known as `name` has had, its current id first. */
export function allExtensionKeys(name: string): string[] {
  const current = currentExtensionKey(name);
  return [
    current,
    ...EXTENSION_RENAMES.filter((rename) => rename.to === current).map((rename) => rename.from),
  ];
}

/** True when `name` is an id that was renamed away (`gptragent`). */
export function isRenamedAway(name: string): boolean {
  const key = toKey(name);
  return EXTENSION_RENAMES.some((rename) => rename.from === key);
}

/** Whether two names are ids of the same extension, before or after a rename. */
export function sameExtension(a: string, b: string): boolean {
  return currentExtensionKey(a) === currentExtensionKey(b);
}
