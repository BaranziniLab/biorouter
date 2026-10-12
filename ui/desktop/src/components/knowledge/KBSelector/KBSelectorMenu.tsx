import { useEffect, useId, useState } from 'react';
import { Check, FolderPlus, Settings } from '../../icons/app-icons';
import { PrivacyBadge } from '../../ui/PrivacyBadge';
import { Button } from '../../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from '../../ui/command';
import { knowledgeCopy } from '../copy';
import { useKnowledge } from '../KnowledgeContext';
import { KbDot } from '../KbDot';

interface Props {
  /** Switch to the manager dialog. */
  onManage: () => void;
  /** Start a create flow (the manager owns the form). */
  onCreate: () => void;
  /** Picking a base closes the picker. */
  onClose: () => void;
}

/**
 * The KB picker: an anchored, searchable list of the bases this chat can see.
 *
 * This is the half of the old 493-line `KBSelectorPalette` that answers "which
 * base am I pointed at" (ui-spec §4.1). The other half — creating, renaming,
 * exporting, deleting and changing which bases are in the chat at all — is
 * `KBManagerDialog`, because a 760px modal is not what clicking a selector
 * should open.
 *
 * ⌘K in the Knowledge view opens THIS, not the manager: ⌘K has always meant
 * "switch base".
 *
 * ⚠ The list is `visibleBases`, not `bases`. A base hidden from this chat is
 * not a thing the chat can be pointed at, so offering it here would present a
 * choice that silently also un-hides. `Manage bases…` is one row away.
 */
export function KBSelectorMenu({ onManage, onCreate, onClose }: Props) {
  const {
    visibleBases,
    primaryKbId,
    defaultPrimaryKb,
    canFollowDefaultPrimary,
    followDefaultPrimary,
    refreshDefaultPrimary,
    refresh,
    setPrimaryKbId,
  } = useKnowledge();
  const [query, setQuery] = useState('');
  const followHintId = useId();

  useEffect(() => {
    // Re-read on open: a base created in chat, and a machine-wide default moved
    // in another chat, are both invisible here otherwise until a full reload.
    void refresh();
    void refreshDefaultPrimary();
  }, [refresh, refreshDefaultPrimary]);

  const needle = query.trim().toLowerCase();
  const filtered = needle
    ? visibleBases.filter(
        (base) => base.name.toLowerCase().includes(needle) || base.id.toLowerCase().includes(needle)
      )
    : visibleBases;

  return (
    <Command
      label="Knowledge bases"
      query={query}
      onQueryChange={setQuery}
      className="max-h-[min(420px,60vh)]"
    >
      <CommandInput
        data-testid="knowledge-kb-search"
        placeholder="Search knowledge bases"
        aria-label="Search knowledge bases"
        autoFocus
      />

      {/*
        The way back to the default. Deliberately NOT a third state on every row
        — a row is "in this chat" plus "make it primary", and a third control
        there would make the invariant look violable. One notice about the whole
        chat, shown only while the chat holds a primary of its own: a chat
        already following the default has nothing to inherit (DR-12).
      */}
      {canFollowDefaultPrimary && defaultPrimaryKb && (
        // One line (spec 3.12): the state, and the way back. What the button
        // will do is its Tooltip and its accessible description, not a
        // paragraph inside a 256px popover.
        <div className="flex flex-none items-center justify-between gap-2 border-b border-border-subtle py-1 pl-3 pr-1">
          <span className="min-w-0 truncate text-supporting text-text-muted">
            {knowledgeCopy.selector.notFollowing}
          </span>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                data-testid="knowledge-kb-follow-default"
                type="button"
                variant="ghost"
                size="sm"
                className="min-w-0 shrink"
                aria-describedby={followHintId}
                onClick={followDefaultPrimary}
              >
                <span className="truncate">
                  {knowledgeCopy.selector.useDefault(defaultPrimaryKb.name)}
                </span>
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom">
              {knowledgeCopy.selector.useDefaultHint(defaultPrimaryKb.name)}
            </TooltipContent>
          </Tooltip>
          <span id={followHintId} className="sr-only">
            {knowledgeCopy.selector.useDefaultHint(defaultPrimaryKb.name)}
          </span>
        </div>
      )}

      <CommandList aria-label="Knowledge bases">
        {filtered.length === 0 ? (
          <CommandEmpty>
            <p className="text-body text-text-muted">No knowledge bases match</p>
            <p className="mt-1 text-supporting text-text-muted">Try a different name or id.</p>
          </CommandEmpty>
        ) : (
          <CommandGroup>
            {filtered.map((base) => {
              const isPrimary = primaryKbId === base.id;
              return (
                <CommandItem
                  key={base.id}
                  selected={isPrimary}
                  onSelect={() => {
                    setPrimaryKbId(base.id);
                    onClose();
                  }}
                >
                  <KbDot color={base.color} />
                  <span data-row-name className="min-w-0 flex-1 truncate">
                    {base.name}
                  </span>
                  {/* No format badge on picker rows (spec 3.12): in a 256px
                      popover it was noise, and the band names the selected
                      base's format. The tier stays: it is what a pick commits
                      the chat to. */}
                  {base.tier !== 'public' && <PrivacyBadge tier={base.tier} dense />}
                  {isPrimary && (
                    <Check
                      className="h-icon-row w-icon-row shrink-0 text-text-default"
                      aria-hidden="true"
                    />
                  )}
                </CommandItem>
              );
            })}
          </CommandGroup>
        )}

        <CommandSeparator />
        <CommandGroup>
          <CommandItem onSelect={onManage} data-testid="knowledge-kb-open-manager">
            <Settings className="h-icon-row w-icon-row shrink-0 text-text-muted" aria-hidden />
            <span data-row-name className="min-w-0 flex-1 truncate">
              Manage bases…
            </span>
          </CommandItem>
          <CommandItem onSelect={onCreate} data-testid="knowledge-kb-open-create">
            <FolderPlus className="h-icon-row w-icon-row shrink-0 text-text-muted" aria-hidden />
            <span data-row-name className="min-w-0 flex-1 truncate">
              Create knowledge base…
            </span>
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </Command>
  );
}
