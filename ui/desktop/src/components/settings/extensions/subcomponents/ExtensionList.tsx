import { useId } from 'react';
import ExtensionItem from './ExtensionItem';
import builtInExtensionsData from '../../../../built-in-extensions.json';
import { ExtensionConfig } from '../../../../api';
import { FixedExtensionEntry } from '../../../ConfigContext';
import type { DefaultProvider } from '../ExtensionsSection';
import type { RegistryLoad } from '../../../baam/registry';
import { EmptyState } from '../../../ui/empty-state';
import { InfoTip } from '../../../ui/info-tip';
import { ENTITY_ICONS } from '../../../icons/entity-icons';
import { EXTENSION_ROW_COPY } from '../copy';
import './extension-row.css';

interface ExtensionListProps {
  extensions: FixedExtensionEntry[];
  onToggle: (extension: FixedExtensionEntry) => Promise<boolean | void> | void;
  onConfigure?: (extension: FixedExtensionEntry) => void;
  onRemove?: (extension: FixedExtensionEntry) => void;
  isStatic?: boolean;
  searchTerm?: string;
  /**
   * Pass-through only (issue #56, §14.5). Resolved once in `ExtensionsSection`
   * — a per-row lookup would be one config read and one provider fetch per
   * extension, on a screen that routinely lists twenty.
   */
  defaultProvider?: DefaultProvider | null;
  /** Pass-through only, for the same reason (issue #56, §13.5). */
  catalog?: RegistryLoad | null;
  /**
   * How old the catalogue behind the provenance sentences is, or `null` when it is current.
   * Said once, in the help of the first group's heading, rather than as a line of its own.
   */
  catalogFreshness?: string | null;
}

export default function ExtensionList({
  extensions,
  onToggle,
  onConfigure,
  onRemove,
  isStatic,
  searchTerm = '',
  defaultProvider,
  catalog,
  catalogFreshness,
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

  const groups = [
    { key: 'enabled', title: EXTENSION_ROW_COPY.groups.enabled, entries: sortedEnabledExtensions },
    {
      key: 'available',
      title: EXTENSION_ROW_COPY.groups.available,
      entries: sortedDisabledExtensions,
    },
  ].filter((group) => group.entries.length > 0);

  return (
    <div className="flex flex-col gap-6">
      {groups.map((group, index) => (
        <ExtensionGroup
          key={group.key}
          title={group.title}
          count={group.entries.length}
          // The catalogue's age is about every provenance sentence below, so it is said once, on
          // the first heading on the page.
          note={index === 0 ? catalogFreshness : null}
        >
          {group.entries.map((extension) => (
            <ExtensionItem
              key={extension.name}
              extension={extension}
              onToggle={onToggle}
              onConfigure={onConfigure}
              onRemove={onRemove}
              isStatic={isStatic}
              defaultProvider={defaultProvider}
              catalog={catalog}
            />
          ))}
        </ExtensionGroup>
      ))}

      {/* The Extensions page's empty states: one sentence each, the only content on the page.
          No action on them: the three ways to get an extension are the band's Add menu directly
          above, and repeating one here would make the same act look like two different offers. */}
      {extensions.length === 0 && (
        <EmptyState
          icon={ENTITY_ICONS.extension}
          title={EXTENSION_ROW_COPY.emptyTitle}
          description={EXTENSION_ROW_COPY.emptyDescription}
        />
      )}
      {extensions.length > 0 && groups.length === 0 && (
        <EmptyState
          icon={ENTITY_ICONS.extension}
          title={EXTENSION_ROW_COPY.noMatchTitle}
          description={EXTENSION_ROW_COPY.noMatchDescription}
          compact
        />
      )}
    </div>
  );
}

/**
 * One group: a caps label carrying its count (Crew's form, no dot), then the rows. The label is
 * inset to the rows' text so the two share a left edge.
 */
function ExtensionGroup({
  title,
  count,
  note,
  children,
}: {
  title: string;
  count: number;
  note?: string | null;
  children: React.ReactNode;
}) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="min-w-0">
      <div className="br-ext-group-label">
        <h2 id={headingId} className="truncate text-caps text-text-muted">
          {title} <span className="tabular-nums">{count}</span>
        </h2>
        {note ? (
          <InfoTip
            label={EXTENSION_ROW_COPY.catalogName}
            help={EXTENSION_ROW_COPY.catalogNote(note)}
          />
        ) : null}
      </div>
      <div className="biorouter-list-shell">{children}</div>
    </section>
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
