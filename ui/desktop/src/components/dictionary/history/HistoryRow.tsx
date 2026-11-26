import { memo, useId, useRef } from 'react';
import type { DictationHistoryEntryView } from '../../../api';
import { Copy, Mic, withPrivateBadge } from '../../icons/app-icons';
import { IconAction, RowActions, RowContextMenu, type RowActionItem } from '../../ui/row-actions';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { DICTIONARY_LABELS } from '../copy';
import {
  HISTORY_ENTRY_ATTRIBUTE,
  HISTORY_SESSION_ATTRIBUTE,
  HISTORY_SURFACE_ATTRIBUTE,
  HISTORY_TIER_ATTRIBUTE,
} from '../termOrigin';
import {
  expandedLine,
  hasSelectionWithin,
  historyMeta,
  selectionWithin,
  tierChangeReason,
  type HistorySettledKind,
} from './historyRows';

const LABELS = DICTIONARY_LABELS.history;

/** The microphone with the lock badge: private is a shape, not a tint. Built once, at module scope. */
const PrivateMic = withPrivateBadge(Mic, 'mic');

export interface HistoryRowActions {
  onCopy: (entry: DictationHistoryEntryView) => void;
  /** Present when the caller may write the dictionary; `selection` is the text picked inside this dictation. */
  onAddToDictionary?: (entry: DictationHistoryEntryView, selection: string) => void;
  onOpenChat: (entry: DictationHistoryEntryView) => void;
  onMakePublic: (entry: DictationHistoryEntryView) => void;
  onMakePrivate: (entry: DictationHistoryEntryView) => void;
  onDelete: (entry: DictationHistoryEntryView) => void;
  /** A click, Enter or Space on the row: expand it in place, or collapse it again. */
  onToggle: (id: string) => void;
}

export interface HistoryRowProps extends HistoryRowActions {
  entry: DictationHistoryEntryView;
  expanded: boolean;
  /** The person acted on this row while the pointer held the list: it says so, in place. */
  settled?: HistorySettledKind;
  /** `authority.write`: delete is offered but disabled where it is refused. */
  writeAllowed: boolean;
  /** `authority.tier_change` (and managed policy): the tier item is disabled with its reason. */
  tierChangeAllowed: boolean;
  tierChangeReason?: string | null;
  /** The time the meta line is read against, so one render reads every row alike. */
  now: number;
}

/**
 * One dictation ("Rows" in `history-design.md` 10.2): the text in the default ink at the body
 * size, clamped to two lines, then one muted line with the microphone (the lock badge when
 * private), the time, where it went and how long it was: "3:39 PM · Chat · 14 s". Every collapsed
 * row has one height, whatever its text, so the list can draw only the rows near the viewport.
 *
 * Hover and focus show **Copy** and the `⋯` menu, which right click, Shift+F10 and the
 * ContextMenu key also open. A right click on text selected inside the dictation is let through
 * to the app's own menu, whose **Add to Biorouter dictionary…** then wins there.
 *
 * A click, Enter or Space expands the row in place to the full, selectable text and one muted
 * line that names the model; a second click collapses it. A click that ends a selection inside
 * the row does neither. Hover never expands: hover peeks, click pins.
 *
 * The row carries its dictation's tier, verified chat and surface as attributes
 * (`termOrigin.ts`), so the context menu door can tell an add started inside it.
 */
function HistoryRow({
  entry,
  expanded,
  settled,
  writeAllowed,
  tierChangeAllowed,
  tierChangeReason: givenReason,
  now,
  ...actions
}: HistoryRowProps) {
  const bodyId = useId();
  const rowRef = useRef<HTMLDivElement>(null);
  // The selection inside this dictation when the person last pressed in the row: the ⋯ menu's
  // press can clear it before the item is chosen.
  const selectionRef = useRef('');
  const remember = (event: React.SyntheticEvent) => {
    // A press inside the row's own menu (portalled, but still a React child) is not a press in
    // the row: by then the press that opened the menu may have cleared the selection.
    if (!(event.target instanceof Node) || !rowRef.current?.contains(event.target)) return;
    selectionRef.current = selectionWithin(rowRef.current);
  };
  const isPrivate = entry.tier === 'private';
  const meta = historyMeta(entry, now);
  const reason = tierChangeAllowed ? null : tierChangeReason(givenReason, entry);

  const tierItem: RowActionItem = isPrivate
    ? {
        label: LABELS.makePublic,
        onSelect: () => actions.onMakePublic(entry),
        disabled: !tierChangeAllowed,
        testId: 'history-make-public',
      }
    : {
        label: LABELS.makePrivate,
        onSelect: () => actions.onMakePrivate(entry),
        disabled: !tierChangeAllowed,
        testId: 'history-make-private',
      };
  const menu: RowActionItem[] = [
    { label: LABELS.copyText, onSelect: () => actions.onCopy(entry), testId: 'history-copy-text' },
    ...(actions.onAddToDictionary
      ? [
          {
            label: LABELS.addToDictionary,
            onSelect: () => actions.onAddToDictionary!(entry, selectionRef.current),
            testId: 'history-add-to-dictionary',
          },
        ]
      : []),
    ...(entry.session_id
      ? [
          {
            label: LABELS.openChat,
            onSelect: () => actions.onOpenChat(entry),
            testId: 'history-open-chat',
          },
        ]
      : []),
    { kind: 'separator' },
    tierItem,
    ...(reason ? [{ kind: 'label' as const, label: reason }] : []),
    {
      label: LABELS.delete,
      onSelect: () => actions.onDelete(entry),
      disabled: !writeAllowed,
      destructive: true,
      testId: 'history-delete',
    },
  ];

  const toggle = () => {
    // A click that ends a selection inside the dictation is the person reading, not asking.
    if (hasSelectionWithin(rowRef.current)) return;
    actions.onToggle(entry.id);
  };

  const sessionAttribute = entry.session_id
    ? { [HISTORY_SESSION_ATTRIBUTE]: entry.session_id }
    : {};

  return (
    <RowContextMenu items={menu} disabled={!!settled}>
      <div
        ref={rowRef}
        className="biorouter-list-row br-history-row flex items-start gap-3 px-3 py-2.5"
        data-history-row={entry.id}
        data-expanded={expanded ? 'true' : undefined}
        data-settled={settled}
        aria-busy={settled ? true : undefined}
        onPointerDownCapture={remember}
        onKeyDownCapture={remember}
        {...{
          [HISTORY_ENTRY_ATTRIBUTE]: entry.id,
          [HISTORY_TIER_ATTRIBUTE]: entry.tier,
          [HISTORY_SURFACE_ATTRIBUTE]: entry.surface,
        }}
        {...sessionAttribute}
      >
        <div
          id={bodyId}
          role="button"
          tabIndex={0}
          aria-expanded={expanded}
          className="br-history-body min-w-0 flex-1"
          data-testid="history-row-body"
          onClick={toggle}
          onKeyDown={(event) => {
            if (event.target !== event.currentTarget) return;
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              actions.onToggle(entry.id);
            }
          }}
          onContextMenu={(event) => {
            // Text selected inside the dictation: the app's own menu, and its dictionary door, win.
            if (hasSelectionWithin(event.currentTarget)) event.stopPropagation();
          }}
        >
          <p className="br-history-text text-body text-text-default" data-testid="history-row-text">
            {entry.text}
          </p>
          {expanded ? (
            <p
              className="br-history-meta text-supporting text-text-muted"
              data-testid="history-row-expanded-line"
            >
              {expandedLine(entry)}
            </p>
          ) : (
            <p
              className="br-history-meta flex min-w-0 items-center gap-1 text-supporting text-text-muted"
              data-testid="history-row-meta"
            >
              <Tooltip>
                <TooltipTrigger asChild>
                  <span
                    className="br-history-glyph"
                    role="img"
                    aria-label={isPrivate ? LABELS.privateDictation : LABELS.publicDictation}
                    data-testid={isPrivate ? 'history-row-private' : 'history-row-public'}
                  >
                    {isPrivate ? <PrivateMic aria-hidden /> : <Mic aria-hidden />}
                  </span>
                </TooltipTrigger>
                <TooltipContent>{LABELS.tier[entry.tier]}</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <time dateTime={entry.created_at} className="shrink-0">
                    {meta.time}
                  </time>
                </TooltipTrigger>
                <TooltipContent>{meta.stamp}</TooltipContent>
              </Tooltip>
              <span aria-hidden>·</span>
              <span className="truncate">{meta.surface}</span>
              <span aria-hidden>·</span>
              <span className="shrink-0">{meta.length}</span>
            </p>
          )}
        </div>
        {settled ? (
          <span className="shrink-0 text-supporting text-text-muted" role="status">
            {LABELS.settled[settled]}
          </span>
        ) : (
          <RowActions
            primary={
              <IconAction
                icon={Copy}
                label={LABELS.copy}
                onSelect={() => actions.onCopy(entry)}
                testId="history-row-copy"
              />
            }
            menu={menu}
            menuLabel={LABELS.moreActions}
            menuTestId="history-row-menu"
          />
        )}
      </div>
    </RowContextMenu>
  );
}

/**
 * Memoized: the tab passes callbacks that keep their identity, so a drawn row renders again only
 * when its entry or its state changes.
 */
export default memo(HistoryRow);
