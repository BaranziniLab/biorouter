import builtInExtensionsData from '../../../built-in-extensions.json';
import { FALLBACK_REGISTRY, marketplaceEntryFor, type BaamRegistry } from '../../baam/registry';

/**
 * The name a person reads for an extension, wherever Biorouter says one (principle 10: names,
 * not ids).
 *
 * An extension reaches the interface under three spellings, and only the last is ever fit to
 * show: the config KEY the daemon reports (`cdwagent`, `computercontroller`), the config entry's
 * own name, and the name its catalogue gives it (`CDWAgent`, `Biorouter Copilot`). Title-casing
 * the key, which every caller used to do alone, reads `Cdwagent failed to load` on the Extensions
 * page, `Failed: Cdwagent` in the load toast and `Computercontroller, Cdwagent` in a History row,
 * while the same extensions are `CDWAgent` and `Biorouter Copilot` in the marketplace and in the
 * composer. One function now decides, so the surfaces cannot name the same thing three ways.
 *
 * It lives in its own module, apart from the Extensions list, so a toast or a History row can ask
 * it without importing the page that renders the rows.
 */

/**
 * Platform capabilities are compiled into the agent and have no catalogue entry, so their
 * display name is stated here. The key is the lowercased name with its whitespace dropped.
 *
 * `workspace` is the daemon's `Workspace` capability, which the daemon itself calls Workspace
 * Control (`bundled_extension_display_name`) and so does Settings > Chat > Capabilities
 * (`capabilities.ts`). Without the entry a load toast or a History row said "Workspace" for the
 * same thing.
 */
const PLATFORM_EXTENSION_DISPLAY_NAMES: Record<string, string> = {
  chatrecall: 'Chat Recall',
  workspace: 'Workspace Control',
};

/** A key read as words, each capitalised (`code_execution` is `Code Execution`). */
function readAsWords(name: string): string {
  return name
    .split(/[-_]/) // Split on hyphens and underscores
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

/** The name a platform capability is given, by its key, or `undefined`. */
function platformNameFor(name: string): string | undefined {
  return PLATFORM_EXTENSION_DISPLAY_NAMES[name.toLowerCase().replace(/\s+/g, '')];
}

/**
 * The last resort: a platform capability's stated name, else a key read as words
 * (`code_execution` is `Code Execution`).
 */
export function formatExtensionName(name: string): string {
  return platformNameFor(name) ?? readAsWords(name);
}

/**
 * What a caller knows about an extension. A bare string is a key with nothing else attached (a
 * load failure, a toast); an object is a config entry, which may also state its own display name.
 */
export interface ExtensionNameEntry {
  name: string;
  type?: string;
  display_name?: string | null;
}

export type ExtensionNameSource = string | ExtensionNameEntry;

/** Case, spaces, hyphens and underscores do not tell two spellings of one key apart. */
function squash(name: string): string {
  return name.toLowerCase().replace(/[\s_-]+/g, '');
}

/** The bundled capability a key names, by its id or by its name, or `null`. */
function builtInNameFor(key: string): string | null {
  const wanted = squash(key);
  const match = builtInExtensionsData.find(
    (entry) => squash(entry.id) === wanted || squash(entry.name) === wanted
  );
  return match?.name ?? null;
}

/**
 * The display name for an extension, from the most specific source that has one:
 *
 *   1. the entry's own `display_name`, when it is a bundled one that states it;
 *   2. the platform names (`Chat Recall`, `Workspace Control`), never for an installed entry;
 *   3. the bundled capabilities' catalogue (`computercontroller` is `Biorouter Copilot`);
 *   4. the marketplace catalogue (`cdwagent` is `CDWAgent`), for anything that is not bundled;
 *   5. the key read as words, only when nothing above knows it.
 *
 * The marketplace lookup is for prose only and is as loose as `marketplaceEntryFor` is, so it
 * is never a reason to treat an extension as a different one. It reads the snapshot bundled with
 * the app unless the caller holds a fresher catalogue: a name changes far less often than a
 * version, and a synchronous answer is what lets a toast name its extension at all.
 */
export function extensionDisplayName(
  source: ExtensionNameSource,
  registry: BaamRegistry = FALLBACK_REGISTRY
): string {
  const entry: ExtensionNameEntry = typeof source === 'string' ? { name: source } : source;
  const type = entry.type;
  const bundled = type === 'builtin' || type === 'platform';
  // A string, or an entry that does not say what it is, may be either; an entry that says it is
  // an installed one (stdio, http, inline) is never a bundled capability, whatever it is called.
  const installed = type !== undefined && !bundled;

  if (type === 'builtin' && entry.display_name) {
    return formatExtensionName(entry.display_name);
  }

  if (!installed) {
    // A platform name is a bundled capability's, so an entry that says it is an installed one is
    // not given it: `workspace` is as likely to be somebody's own MCP server as the capability.
    const platform = platformNameFor(entry.name);
    if (platform) return platform;

    const builtIn = builtInNameFor(entry.name);
    if (builtIn) return builtIn;
  }

  if (!bundled) {
    const listed = marketplaceEntryFor(registry, entry.name)?.name;
    if (listed) return listed;
  }

  return installed ? readAsWords(entry.name) : formatExtensionName(entry.name);
}
