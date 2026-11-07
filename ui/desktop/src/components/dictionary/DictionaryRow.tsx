import { memo, useId } from 'react';
import type { DictionaryEntryView } from '../../api';
import { Check, Dictionary, Pencil, withPrivateBadge } from '../icons/app-icons';
import { IconAction, RowActions, RowContextMenu, type RowActionItem } from '../ui/row-actions';
import { DICTIONARY_LABELS } from './copy';
import { groupOf, lineTwo, usesMeta, type SettledKind } from './rows';

/** The dictionary glyph with the lock badge: private is a shape, not a tint. */
const PrivateTerm = withPrivateBadge(Dictionary, 'PrivateDictionaryTerm');

export interface DictionaryRowActions {
  onEdit: (entry: DictionaryEntryView) => void;
  onCopy: (entry: DictionaryEntryView) => void;
  /** Present when the row names the chat it came from (`suggested_in`). */
  onOpenChat?: (entry: DictionaryEntryView) => void;
  onMakePublic: (entry: DictionaryEntryView) => void;
  onMakePrivate: (entry: DictionaryEntryView) => void;
  onDelete: (entry: DictionaryEntryView) => void;
  onAccept: (entry: DictionaryEntryView) => void;
  onDismiss: (entry: DictionaryEntryView) => void;
  onKeep: (entry: DictionaryEntryView) => void;
  onRemove: (entry: DictionaryEntryView) => void;
}

export interface DictionaryRowProps extends DictionaryRowActions {
  entry: DictionaryEntryView;
  /** The person acted on this row while the pointer held the list: it says so, in place. */
  settled?: SettledKind;
  /** Writes are refused here (managed policy, or a backend from before the dictionary). */
  readOnly?: boolean;
  /** Whether a tier change is admitted (`authority.tier_change.allowed`). */
  tierChangeAllowed: boolean;
}

/**
 * One term ("Rows" in `docs/dictation/dictionary.md`): the written form in
 * `text-label` with the lock badge when private, one muted second line, then
 * the meta ("12 uses") at rest, which gives way to one primary `IconAction` and
 * the `⋯` menu on hover or focus. The same menu opens on right click,
 * Shift+F10 and the ContextMenu key.
 *
 * | Row | Primary | Menu |
 * |---|---|---|
 * | Active | Edit | Edit…, Copy term, Open the chat it came from, Make public… or Make private…, Delete… |
 * | Suggested | Accept | Edit…, Copy term, Open the chat it came from, Dismiss |
 * | Added by the agent, unreviewed | Keep | Edit…, Copy term, Open the chat it came from, Remove |
 *
 * Memoized: the page passes callbacks that keep their identity, so a drawn row
 * renders again only when its entry or its state changes.
 */
export default memo(DictionaryRow);

function DictionaryRow({
  entry,
  settled,
  readOnly = false,
  tierChangeAllowed,
  ...actions
}: DictionaryRowProps) {
  const titleId = useId();
  const kind = groupOf(entry);
  const term = entry.written;
  const name = (action: string) => DICTIONARY_LABELS.rowAction(action, term);

  const common: RowActionItem[] = [
    ...(readOnly
      ? []
      : [{ label: DICTIONARY_LABELS.editMenu, onSelect: () => actions.onEdit(entry) }]),
    { label: DICTIONARY_LABELS.copyTerm, onSelect: () => actions.onCopy(entry) },
    ...(entry.suggested_in && actions.onOpenChat
      ? [{ label: DICTIONARY_LABELS.openChat, onSelect: () => actions.onOpenChat!(entry) }]
      : []),
  ];

  let primary: React.ReactNode = null;
  let menu: RowActionItem[];
  if (kind === 'suggested') {
    primary = readOnly ? null : (
      <IconAction
        icon={Check}
        label={name(DICTIONARY_LABELS.accept)}
        tooltip={DICTIONARY_LABELS.accept}
        onSelect={() => actions.onAccept(entry)}
        testId="dictionary-row-accept"
      />
    );
    menu = [
      ...common,
      { kind: 'separator' },
      { label: DICTIONARY_LABELS.dismiss, onSelect: () => actions.onDismiss(entry) },
    ];
  } else if (kind === 'agent') {
    primary = readOnly ? null : (
      <IconAction
        icon={Check}
        label={name(DICTIONARY_LABELS.keep)}
        tooltip={DICTIONARY_LABELS.keep}
        onSelect={() => actions.onKeep(entry)}
        testId="dictionary-row-keep"
      />
    );
    menu = [
      ...common,
      { kind: 'separator' },
      { label: DICTIONARY_LABELS.remove, onSelect: () => actions.onRemove(entry) },
    ];
  } else {
    primary = readOnly ? null : (
      <IconAction
        icon={Pencil}
        label={name(DICTIONARY_LABELS.edit)}
        tooltip={DICTIONARY_LABELS.edit}
        onSelect={() => actions.onEdit(entry)}
        testId="dictionary-row-edit"
      />
    );
    const tierItem: RowActionItem =
      entry.tier === 'private'
        ? {
            label: DICTIONARY_LABELS.makePublic,
            onSelect: () => actions.onMakePublic(entry),
            disabled: !tierChangeAllowed,
          }
        : {
            label: DICTIONARY_LABELS.makePrivate,
            onSelect: () => actions.onMakePrivate(entry),
            disabled: !tierChangeAllowed,
          };
    menu = [
      ...common,
      { kind: 'separator' },
      ...(readOnly ? [] : [tierItem]),
      {
        label: DICTIONARY_LABELS.delete,
        onSelect: () => actions.onDelete(entry),
        destructive: true,
        testId: 'dictionary-row-delete',
      },
    ];
  }

  const meta = settled ? DICTIONARY_LABELS.settled[settled] : usesMeta(entry);

  return (
    <RowContextMenu items={menu} disabled={!!settled}>
      <div
        className="biorouter-list-row br-dictionary-row flex items-center gap-3 px-3 py-2.5"
        data-dictionary-row={entry.id}
        data-dictionary-group={kind}
        data-settled={settled}
        aria-busy={settled ? true : undefined}
      >
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-1.5">
            <p id={titleId} className="min-w-0 truncate text-label text-text-default">
              {term}
            </p>
            {entry.tier === 'private' ? (
              <PrivateTerm
                className="br-dictionary-private-glyph"
                role="img"
                aria-label={DICTIONARY_LABELS.privateTerm}
                data-testid="dictionary-row-private"
              />
            ) : null}
          </div>
          <p
            className="truncate text-supporting text-text-muted"
            data-testid="dictionary-row-line-two"
          >
            {lineTwo(entry)}
          </p>
        </div>
        {settled ? (
          <span className="shrink-0 text-supporting text-text-muted" role="status">
            {meta}
          </span>
        ) : (
          <RowActions
            primary={primary}
            menu={menu}
            meta={meta ?? undefined}
            menuLabel={DICTIONARY_LABELS.moreActions(term)}
          />
        )}
      </div>
    </RowContextMenu>
  );
}
