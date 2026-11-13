import * as React from 'react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  skillCatalogToggleKey,
  useSkillCatalog,
  type SkillCatalogEntry,
} from '../skills/useSkillCatalog';
import { rankCatalogEntries } from '../skills/searchCatalog';
import { rankEntries, SKILL_NOISE, Weight, type SearchField } from '../baam/search';
import { useConfig } from '../ConfigContext';
import { contextConfigKey, type ContextMeta } from '../settings/contexts/contexts';
import { toastService } from '../../toasts';
import BuiltInBadge from '../ui/BuiltInBadge';
import { uiCopy } from '../ui/copy';
import { SKILLS_COPY, TOOLS_COPY } from './copy';
import { PickerDisclosure, PickerPanel, PickerRow, useBuiltInDisclosure } from './PickerPanel';

/** What the skills list needs from its picker state (see `useSkillPicker`). */
export interface SkillPicker {
  sessionId: string | null;
  /** Enabled skills, the chip's share of the Tools count. */
  activeCount: number;
  /**
   * Whether `activeCount` is this chat's: the catalog has answered (or failed)
   * since the chat came into view. Until then the Tools chip shows no number
   * (V2-R7-3).
   */
  answered: boolean;
  query: string;
  setQuery: (query: string) => void;
  entries: SkillCatalogEntry[];
  toggle: (entry: SkillCatalogEntry) => void;
  bulk: { label: string; onClick: () => void } | null;
  emptyText: string;
  /**
   * The shipped Contexts, behind a disclosure after the installed skills.
   * Optional so a picker built by hand need not describe them; absent means the
   * section is not drawn (in a chat, until its catalog has answered).
   */
  contexts?: BuiltInContexts;
}

/** The "Built-in contexts" section of the list (owner, 2026-11-13). */
export interface BuiltInContexts {
  /** The Contexts the search matches; `CONTEXTS` order while browsing. */
  rows: ContextRow[];
  /** How many are on, whatever the search. */
  onCount: number;
  expanded: boolean;
  setExpanded: (expanded: boolean) => void;
  toggle: (row: ContextRow) => void;
}

export interface ContextRow {
  context: ContextMeta;
  /**
   * In a chat, the catalog row the switch writes (a skill, or the
   * `knowledge-bases` bundle); `null` when the catalog lists none. Unused on
   * Home, where the switch is the Context's config key.
   */
  entry: SkillCatalogEntry | null;
  checked: boolean;
  /** In a chat, nothing in its catalog to switch. */
  unavailable: boolean;
  /** A Home write for this row is still in flight. */
  busy: boolean;
}

/** What a person reads on a Context row, then its id: the matcher's fields. */
function contextSearchFields(row: ContextRow): SearchField[] {
  return [
    [row.context.label, Weight.Name],
    [row.context.id, Weight.Name],
    [row.context.description, Weight.Prose],
  ];
}

/**
 * The composer's skill list: state and toggles, without any trigger.
 *
 * ⚠ **Every toggle here goes to the daemon and waits for its answer** (#113).
 * The session branch used to write React state and raise a success toast
 * reading "<skill> enabled for this chat" without calling anything. A control
 * that reports intent as if it were state is worse than no control, so a
 * refusal puts the switch back and says why. There is no success toast at all
 * now (decision 6.9): the switch is the confirmation.
 *
 * The inventory is fetched rather than scanned, for the reason recorded on
 * `useSkillCatalog`: a three-root scan omitted skills bundled inside installed
 * extensions, which the backend has always loaded.
 *
 * @param open whether the list is on screen. Opening it asks the daemon to
 *   rescan, the cheapest honest moment to notice an install made elsewhere.
 */
export function useSkillPicker(sessionId: string | null, { open }: { open: boolean }): SkillPicker {
  const [searchQuery, setSearchQuery] = useState('');
  const catalog = useSkillCatalog(sessionId);
  const { entries, reload, setEnabled } = catalog;

  useEffect(() => {
    if (open) void reload(true);
  }, [open, reload]);

  const applyToggle = useCallback(
    async (keys: string[], enabled: boolean) => {
      const result = await setEnabled(keys, enabled);
      if (result.ok) return;
      // The reason travels back with the result rather than through hook
      // state, which this callback's captured render would have read stale.
      toastService.error({
        title: SKILLS_COPY.updateFailedTitle,
        msg: SKILLS_COPY.updateFailed(result.error),
      });
    },
    [setEnabled]
  );

  const toggle = useCallback(
    (entry: SkillCatalogEntry) => {
      void applyToggle([skillCatalogToggleKey(entry)], !entry.enabled);
    },
    [applyToggle]
  );

  // One matcher, shared with Settings -> Skills, the Browse modals and the
  // model's own search — see `skills/searchCatalog.ts`. This filter used to ask
  // whether the WHOLE query occurred inside one field (QA finding F5), so a
  // phrase naming two installed skills found neither. The list is flat, so the
  // ranking reaches it directly: best match first, catalog order when nothing
  // is typed.
  //
  // ⚠ It also feeds "Enable all", which writes every row left on screen — so a
  // filter that leaks (`R` matching the letter anywhere) is a bulk write over
  // rows the user never asked about, not just a long list.
  const filteredEntries = useMemo(
    () => rankCatalogEntries(entries, searchQuery).hits.map((hit) => hit.entry),
    [entries, searchQuery]
  );

  const activeCount = useMemo(() => entries.filter((e) => e.enabled).length, [entries]);

  // Whether the catalog has answered for THIS chat (V2-R7-3). `useSkillCatalog`
  // keeps what it last held (the previous chat's view, or an empty one at
  // first) until the new chat's read commits, and `loading` cannot say which
  // read settled. So the chat counts as answered once a view, or an error,
  // newer than the one it came into view with has been committed.
  const [openedOn, setOpenedOn] = useState(() => ({ sessionId, entries, error: catalog.error }));
  if (openedOn.sessionId !== sessionId) {
    setOpenedOn({ sessionId, entries, error: catalog.error });
  }
  const answered =
    openedOn.sessionId === sessionId &&
    (entries !== openedOn.entries || (catalog.error !== null && catalog.error !== openedOn.error));
  const visibleEnabledCount = useMemo(
    () => filteredEntries.filter((e) => e.enabled).length,
    [filteredEntries]
  );

  const handleBulkToggle = useCallback(() => {
    if (filteredEntries.length === 0) return;
    const targetEnabled = visibleEnabledCount === 0;
    const keys = [
      ...new Set(
        filteredEntries.filter((e) => e.enabled !== targetEnabled).map(skillCatalogToggleKey)
      ),
    ];
    if (keys.length === 0) return;
    void applyToggle(keys, targetEnabled);
  }, [applyToggle, filteredEntries, visibleEnabledCount]);

  const emptyText = catalog.error
    ? catalog.error
    : catalog.loading
      ? SKILLS_COPY.loading
      : searchQuery
        ? SKILLS_COPY.noMatch
        : SKILLS_COPY.empty;

  /**
   * The shipped Contexts, each with its state (owner, 2026-11-13).
   *
   * - **In a chat** the switch is the per-chat override, `POST /skills/session`
   *   through `setEnabled`, keyed by the skill's name or, for Knowledge, the
   *   bundle's (`skillCatalogToggleKey`). Its state is the catalog's composed
   *   `effective`, in which this chat's own choice beats Settings → Contexts in
   *   both directions (`compose_state`).
   * - **On Home** there is no chat to hold an override, so the switch writes the
   *   Context's config key, exactly as Settings → Contexts does, and the scope
   *   line already says "New chats". Its state is the machine-wide catalog's
   *   answer, which also honours the key Knowledge inherits from `update-soul`.
   *
   * ⚠ Never part of `activeCount`, "Enable all" or the ranked `entries`: on
   * Home those write `skills-config.json`, whose `disabled[]` would make
   * `loadSkill` refuse a Context (see `contexts.ts`).
   */
  const { config, upsert } = useConfig();
  const [homePending, setHomePending] = useState<ReadonlyMap<string, boolean>>(new Map());
  // Home writes for the same key land in the order they were made.
  const homeQueue = useRef<Promise<unknown>>(Promise.resolve());
  const homeTokens = useRef(new Map<string, number>());
  const catalogAnswered = catalog.generation > 0;

  const contextRows = useMemo<ContextRow[]>(
    () =>
      catalog.contexts.map(({ context, entry }) => {
        if (!sessionId) {
          // Before the catalog answers, the key as Settings reads it: absent is on.
          const stored = (config as Record<string, unknown> | undefined)?.[
            contextConfigKey(context.id)
          ];
          return {
            context,
            entry: null,
            checked:
              homePending.get(context.id) ??
              entry?.enabled ??
              (typeof stored === 'boolean' ? stored : true),
            unavailable: false,
            busy: homePending.has(context.id),
          };
        }
        return {
          context,
          entry,
          checked: entry?.enabled ?? false,
          unavailable: entry === null,
          busy: false,
        };
      }),
    [catalog.contexts, sessionId, homePending, config]
  );

  const toggleContext = useCallback(
    (row: ContextRow) => {
      const next = !row.checked;
      if (sessionId) {
        if (row.entry) void applyToggle([skillCatalogToggleKey(row.entry)], next);
        return;
      }
      const id = row.context.id;
      const token = (homeTokens.current.get(id) ?? 0) + 1;
      homeTokens.current.set(id, token);
      setHomePending((prev) => new Map(prev).set(id, next));
      const run = async () => {
        try {
          await upsert(contextConfigKey(id), next, false);
          // The catalog reads the key fresh on every view.
          await reload();
        } catch (error) {
          toastService.error({
            title: SKILLS_COPY.updateFailedTitle,
            msg: SKILLS_COPY.updateFailed(
              error instanceof Error ? error.message : 'the request failed.'
            ),
          });
        } finally {
          if (homeTokens.current.get(id) === token) {
            setHomePending((prev) => {
              const rest = new Map(prev);
              rest.delete(id);
              return rest;
            });
          }
        }
      };
      const queued = homeQueue.current.then(run, run);
      homeQueue.current = queued;
    },
    [applyToggle, reload, sessionId, upsert]
  );

  const matchingContexts = useMemo(
    () =>
      rankEntries(searchQuery, SKILL_NOISE, contextRows, contextSearchFields).hits.map(
        (hit) => hit.entry
      ),
    [contextRows, searchQuery]
  );
  const [contextsExpanded, setContextsExpanded] = useBuiltInDisclosure(
    open,
    searchQuery.trim() !== '' && matchingContexts.length > 0
  );

  return {
    sessionId,
    activeCount,
    answered,
    query: searchQuery,
    setQuery: setSearchQuery,
    entries: filteredEntries,
    toggle,
    bulk:
      filteredEntries.length > 0
        ? {
            label:
              visibleEnabledCount === 0
                ? SKILLS_COPY.enableAll(filteredEntries.length)
                : SKILLS_COPY.disableAll(visibleEnabledCount),
            onClick: handleBulkToggle,
          }
        : null,
    emptyText,
    // In a chat, not before its catalog has answered: until then every row
    // would read as missing.
    contexts:
      sessionId && !catalogAnswered
        ? undefined
        : {
            rows: matchingContexts,
            onCount: contextRows.filter((row) => row.checked).length,
            expanded: contextsExpanded,
            setExpanded: setContextsExpanded,
            toggle: toggleContext,
          },
  };
}

/** The skills list inside the Tools popover. */
export function SkillsPanel({
  picker,
  inputRef,
}: {
  picker: SkillPicker;
  inputRef?: React.Ref<HTMLInputElement>;
}) {
  const contexts = picker.contexts;
  // Drawn while browsing, and under a search only when it matches a Context.
  const showContexts = !!contexts && (picker.query.trim() === '' || contexts.rows.length > 0);
  // A search that found only Contexts is not "No skills found".
  const onlyContextsMatch =
    showContexts && picker.query.trim() !== '' && picker.entries.length === 0;
  return (
    <PickerPanel
      data-testid="tools-skills-panel"
      listLabel={SKILLS_COPY.listLabel}
      searchPlaceholder={SKILLS_COPY.search}
      query={picker.query}
      onQueryChange={picker.setQuery}
      scopeLabel={picker.sessionId ? TOOLS_COPY.scope.thisChat : TOOLS_COPY.scope.newChats}
      scopeHelp={picker.sessionId ? SKILLS_COPY.helpChat : SKILLS_COPY.helpNewChats}
      bulk={picker.bulk}
      emptyText={onlyContextsMatch ? null : picker.emptyText}
      inputRef={inputRef}
      trailing={
        showContexts ? (
          <PickerDisclosure
            data-testid="tools-builtin-contexts"
            label={SKILLS_COPY.contextsLabel}
            summary={SKILLS_COPY.contextsSummary(contexts.onCount)}
            open={contexts.expanded}
            onOpenChange={contexts.setExpanded}
          >
            {contexts.rows.map((row) => (
              <PickerRow
                key={row.context.id}
                name={row.context.label}
                checked={row.checked}
                disabled={row.unavailable}
                busy={row.busy}
                onToggle={() => contexts.toggle(row)}
                note={row.unavailable ? SKILLS_COPY.contextUnavailable : undefined}
                description={row.context.description}
              />
            ))}
          </PickerDisclosure>
        ) : undefined
      }
    >
      {picker.entries.map((entry) => (
        <SkillRow key={entry.key} entry={entry} onToggle={picker.toggle} />
      ))}
    </PickerPanel>
  );
}

function SkillRow({
  entry,
  onToggle,
}: {
  entry: SkillCatalogEntry;
  onToggle: (entry: SkillCatalogEntry) => void;
}) {
  if (entry.kind === 'single') {
    const { skill, enabled } = entry;
    // `label` is the extension's own name when the skill was bundled inside one:
    // provenance the picker could not show while it scanned only three roots.
    const from = skill.source.kind === 'extension' ? skill.source.label : undefined;
    return (
      <PickerRow
        name={skill.name}
        checked={enabled}
        onToggle={() => onToggle(entry)}
        badge={skill.builtin ? <BuiltInBadge /> : undefined}
        meta={from}
        // A shipped skill with no description still explains its badge.
        description={skill.description || (skill.builtin ? uiCopy.builtInHelp : undefined)}
      />
    );
  }

  // A bundle: its count beside the name, its members in the tooltip.
  const { bundle, enabled } = entry;
  return (
    <PickerRow
      name={bundle.displayName}
      checked={enabled}
      onToggle={() => onToggle(entry)}
      meta={SKILLS_COPY.bundleCount(bundle.skills.length)}
      description={SKILLS_COPY.bundleMembers(bundle.skills)}
    />
  );
}
