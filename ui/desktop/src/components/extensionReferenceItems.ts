import type { ExtensionConfig } from '../api';
import type { DisplayItem } from './MentionPopover';
import { CAPABILITIES } from './settings/capabilities/capabilities';

const capabilityKey = (name: string) => name.replace(/[^a-zA-Z0-9]/g, '').toLowerCase();

/** Bundled capabilities can be explicitly selected; other extensions must be active. */
export function extensionReferenceItems(activeExtensions: ExtensionConfig[]): DisplayItem[] {
  const reservedNames = new Set([
    ...CAPABILITIES.flatMap((item) => [capabilityKey(item.key), capabilityKey(item.label)]),
    'autovisualizer',
  ]);
  const items: DisplayItem[] = CAPABILITIES.map((capability) => ({
    name: `ext:${capability.label}`,
    extra: capability.description,
    itemType: 'Extension',
    relativePath: capability.key,
    builtIn: true,
    searchTerms: [
      `ext:${capability.key}`,
      ...(capability.key === 'autovisualiser' ? ['ext:autovisualizer'] : []),
    ],
  }));
  const seen = new Set<string>();
  for (const extension of activeExtensions) {
    // The resolver gives bundled identities precedence; a custom name collision
    // would select the wrong target or be refused instead of naming this extension.
    if (reservedNames.has(capabilityKey(extension.name))) continue;
    const key = extension.name.replace(/\s/g, '').toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    const label =
      extension.type === 'builtin' && extension.display_name
        ? extension.display_name
        : extension.name;
    items.push({
      name: `ext:${label}`,
      extra: extension.description || 'Active extension',
      itemType: 'Extension',
      relativePath: extension.name,
      searchTerms: [`ext:${extension.name}`],
      builtIn: false,
    });
  }
  return items;
}
