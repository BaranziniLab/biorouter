import ExtensionItem from './ExtensionItem';
import builtInExtensionsData from '../../../../built-in-extensions.json';
import { ExtensionConfig } from '../../../../api';
import { FixedExtensionEntry } from '../../../ConfigContext';
import type { DefaultProvider } from '../ExtensionsSection';
import type { RegistryLoad } from '../../../baam/registry';
import { EmptyState } from '../../../ui/empty-state';
import { Puzzle } from '../../../icons/app-icons';

interface ExtensionListProps {
  extensions: FixedExtensionEntry[];
  onToggle: (extension: FixedExtensionEntry) => Promise<boolean | void> | void;
  onConfigure?: (extension: FixedExtensionEntry) => void;
  isStatic?: boolean;
  disableConfiguration?: boolean;
  searchTerm?: string;
  /**
   * Pass-through only (issue #56, §14.5). Resolved once in `ExtensionsSection`
   * — a per-row lookup would be one config read and one provider fetch per
   * extension, on a screen that routinely lists twenty.
   */
  defaultProvider?: DefaultProvider | null;
  /** Pass-through only, for the same reason (issue #56, §13.5). */
  catalog?: RegistryLoad | null;
}

export default function ExtensionList({
  extensions,
  onToggle,
  onConfigure,
  isStatic,
  disableConfiguration: _disableConfiguration,
  searchTerm = '',
  defaultProvider,
  catalog,
}: ExtensionListProps) {
  const matchesSearch = (extension: FixedExtensionEntry): boolean => {
    if (!searchTerm) return true;

    const searchLower = searchTerm.toLowerCase();
    const title = getFriendlyTitle(extension).toLowerCase();
    const name = extension.name.toLowerCase();
    const subtitle = getSubtitle(extension);
    const description = subtitle.description?.toLowerCase() || '';

    return (
      title.includes(searchLower) || name.includes(searchLower) || description.includes(searchLower)
    );
  };

  // Separate enabled and disabled extensions, then filter by search term
  const enabledExtensions = extensions.filter((ext) => ext.enabled && matchesSearch(ext));
  const disabledExtensions = extensions.filter((ext) => !ext.enabled && matchesSearch(ext));

  // Sort each group alphabetically by their friendly title
  const sortedEnabledExtensions = [...enabledExtensions].sort((a, b) =>
    getFriendlyTitle(a).localeCompare(getFriendlyTitle(b))
  );
  const sortedDisabledExtensions = [...disabledExtensions].sort((a, b) =>
    getFriendlyTitle(a).localeCompare(getFriendlyTitle(b))
  );

  return (
    <div className="space-y-8">
      {sortedEnabledExtensions.length > 0 && (
        <div>
          <h2 className="text-caps text-text-muted mb-3 flex items-center gap-2">
            <span className="w-1.5 h-1.5 bg-background-success rounded-full flex-shrink-0"></span>
            Default Extensions ({sortedEnabledExtensions.length})
          </h2>
          <div className="biorouter-list-shell">
            {sortedEnabledExtensions.map((extension) => (
              <ExtensionItem
                key={extension.name}
                extension={extension}
                onToggle={onToggle}
                onConfigure={onConfigure}
                isStatic={isStatic}
                defaultProvider={defaultProvider}
                catalog={catalog}
              />
            ))}
          </div>
        </div>
      )}

      {sortedDisabledExtensions.length > 0 && (
        <div>
          <h2 className="text-caps text-text-muted mb-3 flex items-center gap-2">
            <span className="w-1.5 h-1.5 bg-background-strong rounded-full flex-shrink-0"></span>
            Available Extensions ({sortedDisabledExtensions.length})
          </h2>
          <div className="biorouter-list-shell">
            {sortedDisabledExtensions.map((extension) => (
              <ExtensionItem
                key={extension.name}
                extension={extension}
                onToggle={onToggle}
                onConfigure={onConfigure}
                isStatic={isStatic}
                defaultProvider={defaultProvider}
                catalog={catalog}
              />
            ))}
          </div>
        </div>
      )}

      {/* The Extensions page's empty state, and the one vocabulary change made
          in this directory (2026-09-07). `components/settings/extensions/` is
          otherwise still OUT of the vocabulary sweep — it sits in the settings
          root's own `extensions/` exclusion, and sweeping it triples the diff —
          but this particular line is what the Extensions PAGE renders when it
          has nothing to show, and a bare `text-sm` sentence pinned to the top
          left of a 760px column is not an empty state. Rule 4: every in-place
          prose block is a `Note` or the shared `EmptyState`.

          No action on it. The three ways to get an extension (Add, Browse, Add
          Custom) are already in the page header directly above, and repeating
          one of them here would make the same act look like two different
          offers. */}
      {extensions.length === 0 && (
        <EmptyState
          icon={Puzzle}
          title="No extensions yet"
          description="MCP extensions add prompts, resources and tools to every new chat. Browse the marketplace or add one you already have."
        />
      )}
    </div>
  );
}

// Helper functions
const PLATFORM_EXTENSION_DISPLAY_NAMES: Record<string, string> = {
  chatrecall: 'Chat Recall',
};

export function formatExtensionName(name: string): string {
  const normalized = name.toLowerCase().replace(/\s+/g, '');
  const displayName = PLATFORM_EXTENSION_DISPLAY_NAMES[normalized];
  if (displayName) return displayName;

  return name
    .split(/[-_]/) // Split on hyphens and underscores
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

export function getFriendlyTitle(extension: FixedExtensionEntry): string {
  const name = (extension.type === 'builtin' && extension.display_name) || extension.name;
  return formatExtensionName(name);
}

// 'builtin' extensions are the bundled MCP servers; 'platform' extensions
// (todo, chatrecall, code_execution, skills, extensionmanager, ...) are
// compiled into the agent itself. Both ship with Biorouter.
export function isBuiltInExtension(extension: ExtensionConfig): boolean {
  return extension.type === 'builtin' || extension.type === 'platform';
}

function normalizeExtensionName(name: string): string {
  return name.toLowerCase().replace(/\s+/g, '');
}

export function getSubtitle(config: ExtensionConfig) {
  switch (config.type) {
    case 'builtin': {
      const extensionData = builtInExtensionsData.find(
        (ext) => normalizeExtensionName(ext.name) === normalizeExtensionName(config.name)
      );
      // No literal fallback string. "Built-in extension" is not a description
      // of anything: it repeats the badge already beside the title and tells
      // the reader nothing about what the extension does. An entry with no
      // description shows none, which is honest and lets the row collapse.
      return {
        description: extensionData?.description || config.description || null,
        command: null,
      };
    }
    case 'sse':
    case 'streamable_http': {
      // No shouted transport prefix. `command` below already renders the URI on
      // the following line, so "STREAMABLE HTTP extension:" duplicated the
      // transport in capitals and pushed the real description to the right.
      return {
        description: config.description || null,
        command: config.uri || null,
      };
    }

    default:
      return {
        description: config.description || null,
        command: 'cmd' in config ? [config.cmd, ...config.args].join(' ') : null,
      };
  }
}
