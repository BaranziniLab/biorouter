import { useId, useLayoutEffect, useRef, useState } from 'react';
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
  /**
   * A toggle moves its row to the other group, and two things must not go wrong when it does
   * (spec principle 9: layout snaps, and a row never moves under the pointer).
   *
   * 1. **While the pointer is in the list, groups stay as they were when it came in.** The switch
   *    shows the new state at once (the row is handed the live data) and the row relocates when
   *    the pointer leaves, so the rows below it never slide under a pointer that is about to
   *    click one of them. Without a pointer in the list (the keyboard), it moves straight away.
   * 2. **A row that moves keeps the keyboard where it was.** It is re-created under the other
   *    group, which removes the focused switch from the page and, until this, dropped the person
   *    at the top of the document after every Space.
   */
  const [held, setHeld] = useState<ReadonlyMap<string, boolean> | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const focusMemo = useRef<{ name: string; scroll: boolean } | null>(null);
  const groupedEnabled = (extension: FixedExtensionEntry): boolean =>
    held?.get(extension.name) ?? extension.enabled;

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
  const enabledExtensions = extensions.filter((ext) => groupedEnabled(ext) && matchesSearch(ext));
  const disabledExtensions = extensions.filter((ext) => !groupedEnabled(ext) && matchesSearch(ext));

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

  /** Remember which row's switch holds focus, just before something that can move that row. */
  const rememberFocus = () => {
    const active = document.activeElement;
    focusMemo.current = null;
    if (!(active instanceof HTMLElement) || !listRef.current?.contains(active)) return;
    if (!active.matches('[role="switch"]')) return;
    const name = active.closest<HTMLElement>('[data-extension-name]')?.dataset.extensionName;
    if (!name) return;
    // Scrolling the row back into view is for someone who can no longer see where the keyboard
    // went. A pointer user has just looked away from the list: it must not jump under them.
    let keyboard = false;
    try {
      keyboard = active.matches(':focus-visible');
    } catch {
      /* a host without :focus-visible: treat as pointer */
    }
    focusMemo.current = { name, scroll: keyboard };
  };

  // After the rows have been re-created in their new groups: give the switch its focus back,
  // unless focus has gone somewhere real since (the person moved on, or the row kept it).
  const layoutKey = groups
    .map((group) => `${group.key}:${group.entries.map((entry) => entry.name).join(',')}`)
    .join('|');
  useLayoutEffect(() => {
    const memo = focusMemo.current;
    if (!memo) return;
    focusMemo.current = null;
    const active = document.activeElement;
    if (active && active !== document.body) return;
    const row = Array.from(
      listRef.current?.querySelectorAll<HTMLElement>('[data-extension-name]') ?? []
    ).find((element) => element.dataset.extensionName === memo.name);
    row?.querySelector<HTMLElement>('[role="switch"]')?.focus({ preventScroll: !memo.scroll });
  }, [layoutKey]);

  const handleToggle = (extension: FixedExtensionEntry) => {
    rememberFocus();
    const result = onToggle(extension);
    // A toggle that failed moves nothing, so there is nothing to give focus back for.
    Promise.resolve(result).catch(() => {
      focusMemo.current = null;
    });
    return result;
  };

  const releaseHeldGroups = () => {
    // Only a pointer that leaves with a move waiting needs the focus remembered; a stale memo
    // would take focus back at some later, unrelated move.
    const pending = held
      ? extensions.some((ext) => held.has(ext.name) && held.get(ext.name) !== ext.enabled)
      : false;
    if (pending) rememberFocus();
    setHeld(null);
  };

  return (
    <div
      ref={listRef}
      className="flex flex-col gap-6"
      onPointerEnter={(event) => {
        // A help bubble or a menu opened from a row is portalled out of the list's DOM but is
        // still its React child, so React reports the pointer "entering" the list when it is
        // really over the bubble. Only a pointer on the list's own DOM holds the groups.
        if (!event.currentTarget.contains(event.target as Node)) return;
        setHeld(new Map(extensions.map((ext) => [ext.name, ext.enabled])));
      }}
      onPointerLeave={releaseHeldGroups}
    >
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
              onToggle={handleToggle}
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
      {/* Rows read apart by their own air and the hover wash (R-27), so the shell opts out of the
          shared hairline with the one shared class. Forced colours get the line back from it. */}
      <div className="biorouter-list-shell br-rows-flat">{children}</div>
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
