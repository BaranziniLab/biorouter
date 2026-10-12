import * as React from 'react';
import { useEffect, useMemo, useState } from 'react';
import BuiltInBadge from '../ui/BuiltInBadge';
import { BUILTIN_RECREATED_TITLE, isBuiltinKnowledgeBase } from '../../utils/builtins';
import { useKnowledge } from '../knowledge/KnowledgeContext';
import { KNOWLEDGE_COPY, TOOLS_COPY } from './copy';
import { PickerPanel, PickerRow } from './PickerPanel';

type KnowledgeBase = ReturnType<typeof useKnowledge>['bases'][number];

/** What the knowledge list needs from its picker state (see `useKnowledgePicker`). */
export interface KnowledgePicker {
  /** Visible bases, the chip's share of the Tools count. */
  visibleCount: number;
  /**
   * Whether `visibleCount` stands: the base list has been read, and this
   * scope's selection has settled. Until then the Tools chip shows no number
   * (V2-R7-3).
   */
  answered: boolean;
  query: string;
  setQuery: (query: string) => void;
  bases: KnowledgeBase[];
  isHidden: (id: string) => boolean;
  toggle: (id: string) => void;
  bulk: { label: string; onClick: () => void } | null;
}

/**
 * The composer's knowledge-base list: which bases this chat can see. State and
 * writes come from `KnowledgeContext`, which carries the user-action proof on
 * every selection read and write.
 *
 * @param open whether the list is on screen. Opening it re-reads the daemon, as
 *   the Knowledge view's picker does: a base created from the CLI or another
 *   window changes nothing this renderer can hear.
 */
export function useKnowledgePicker({ open }: { open: boolean }): KnowledgePicker {
  const knowledge = useKnowledge();
  const {
    bases,
    visibleBases,
    hiddenKbIds,
    setHiddenKbIds,
    toggleKbHidden,
    hideAllKnowledgeBases,
    showAllKnowledgeBases,
    refresh,
    loading,
  } = knowledge;
  const [searchQuery, setSearchQuery] = useState('');

  // The list counts once it has been read at all: a later re-read (`loading`
  // again, after every turn) must not take a number already shown away.
  const [basesRead, setBasesRead] = useState(!loading);
  if (!basesRead && !loading) setBasesRead(true);
  // Whether the selection (which bases are hidden) is this scope's answer and
  // not the previous chat's or this renderer's cached guess. Read as optional:
  // `KnowledgeContext` (WS-KNOWLEDGE) gains the field through request
  // WS-COMPOSER-4; until it does there is nothing to wait for.
  const selectionSettled =
    (knowledge as Partial<{ selectionSettled: boolean }>).selectionSettled ?? true;

  useEffect(() => {
    if (open) void refresh();
    // `refresh` changes identity with the selection it re-reads; opening is
    // the event, not a new function.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const filteredBases = useMemo(
    () =>
      bases.filter((base) => base.name.toLowerCase().includes(searchQuery.trim().toLowerCase())),
    [bases, searchQuery]
  );

  const filteredVisibleCount = useMemo(
    () => filteredBases.filter((base) => !hiddenKbIds.includes(base.id)).length,
    [filteredBases, hiddenKbIds]
  );

  const handleBulkToggle = () => {
    if (filteredBases.length === 0) return;

    const targetVisible = filteredVisibleCount === 0;
    if (!searchQuery.trim()) {
      if (targetVisible) {
        showAllKnowledgeBases();
      } else {
        hideAllKnowledgeBases();
      }
      return;
    }

    // One complete set, one write. Toggling id by id looks equivalent but is
    // not: every `toggleKbHidden` derives its next set from the same captured
    // `hiddenKbIds`, so each call overwrites the one before it and only the last
    // base in the filter actually changes — and each is its own daemon
    // round-trip. Bases the search did not match keep whatever they had.
    const filteredIds = new Set(filteredBases.map((base) => base.id));
    const nextHidden = targetVisible
      ? hiddenKbIds.filter((id) => !filteredIds.has(id))
      : [...hiddenKbIds, ...filteredIds];
    setHiddenKbIds(nextHidden);
  };

  return {
    visibleCount: visibleBases.length,
    answered: basesRead && selectionSettled,
    query: searchQuery,
    setQuery: setSearchQuery,
    bases: filteredBases,
    isHidden: (id) => hiddenKbIds.includes(id),
    toggle: toggleKbHidden,
    bulk:
      filteredBases.length > 0
        ? {
            label:
              filteredVisibleCount === 0
                ? KNOWLEDGE_COPY.showAll(filteredBases.length)
                : KNOWLEDGE_COPY.hideAll(filteredVisibleCount),
            onClick: handleBulkToggle,
          }
        : null,
  };
}

/** The knowledge list inside the Tools popover. */
export function KnowledgePanel({
  picker,
  inChat,
  inputRef,
}: {
  picker: KnowledgePicker;
  /** In a chat the list is this chat's; on Home it is the default chats inherit. */
  inChat: boolean;
  inputRef?: React.Ref<HTMLInputElement>;
}) {
  return (
    <PickerPanel
      data-testid="tools-knowledge-panel"
      listLabel={KNOWLEDGE_COPY.listLabel}
      searchPlaceholder={KNOWLEDGE_COPY.search}
      query={picker.query}
      onQueryChange={picker.setQuery}
      scopeLabel={inChat ? TOOLS_COPY.scope.thisChat : TOOLS_COPY.scope.defaultForChats}
      scopeHelp={inChat ? KNOWLEDGE_COPY.helpChat : KNOWLEDGE_COPY.helpDefault}
      bulk={picker.bulk}
      emptyText={picker.query ? KNOWLEDGE_COPY.noMatch : KNOWLEDGE_COPY.empty}
      inputRef={inputRef}
    >
      {picker.bases.map((base) => {
        const hidden = picker.isHidden(base.id);
        return (
          <PickerRow
            key={base.id}
            name={base.name}
            checked={!hidden}
            onToggle={() => picker.toggle(base.id)}
            // One verb pair per popup (#82): the row says VISIBILITY, or the
            // switch reads as the enable/disable toggle the other two lists use.
            label={KNOWLEDGE_COPY.rowLabel(base.name, hidden)}
            badge={isBuiltinKnowledgeBase(base.id) ? <BuiltInBadge /> : undefined}
            // The badge is plain inside a row (the row is the control), so the
            // sentence it used to carry as a title rides on the row's description.
            description={
              isBuiltinKnowledgeBase(base.id)
                ? BUILTIN_RECREATED_TITLE.replace(/\s*\n\s*/g, ' ')
                : undefined
            }
          />
        );
      })}
    </PickerPanel>
  );
}
