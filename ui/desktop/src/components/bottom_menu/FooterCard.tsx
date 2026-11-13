import './pickers.css';
import * as React from 'react';
import { cn } from '../../utils';
import { homeFolder, tildePath } from '../../utils/folderPath';
import { PathSegments } from '../ui/path-segments';
import { Popover, PopoverAnchor, PopoverContent } from '../ui/popover';
import { Button } from '../ui/button';
import {
  FooterCardGroupContext,
  footerCardTriggerProps,
  useFooterCardGroupValue,
  type FooterCard as FooterCardState,
} from './useFooterCard';

/**
 * The composer footer's cards, ONE format for all three (the working folder, the context ring, the
 * cost): the same light popover surface, the same section label, the same reading under it, the
 * same muted detail and note lines, the same rows and the same buttons. Each card is built only
 * from these pieces, so the same kind of information looks the same in each. When a card opens
 * and closes is `useFooterCard`'s; every trigger is a `FooterCardTrigger`.
 *
 * The popover surface, not the tooltip's inverse one: the cards hold buttons, a slider and a field,
 * and InfoTip's rules keep controls off the inverse ground. Load-bearing layout and colour are
 * authored CSS (`.br-footer-card*` in pickers.css), never new Tailwind utilities, because a new
 * utility can silently fail to generate under BIOROUTER_NO_HMR.
 */

/**
 * How close a card may come to the window's edges: off the 44px window-drag band at the top
 * (`--chrome-height`, plus 8px), where a click on a button in the card would be eaten by the drag
 * region, as for the Tools popover (`TOOLS_POPOVER_COLLISION_PADDING`).
 */
export const FOOTER_CARD_COLLISION_PADDING = { top: 52, right: 8, bottom: 8, left: 8 } as const;

/**
 * The footer line's cards, held together: one open at a time, and all of them hanging above the
 * composer card (`clearOf`). `ComposerFooter` renders it round the whole line. Without it, a card
 * works on its own and hangs from its trigger.
 */
export function FooterCardGroup({
  clearOf,
  children,
}: {
  clearOf?: React.RefObject<HTMLElement | null>;
  children: React.ReactNode;
}) {
  const value = useFooterCardGroupValue(clearOf);
  return (
    <FooterCardGroupContext.Provider value={value}>{children}</FooterCardGroupContext.Provider>
  );
}

/**
 * The Popover root and the anchor a card hangs from. CONTROLLED for the item's whole life (#50): a
 * root that flipped from controlled to uncontrolled mid-life is what showed up as a stuck-open
 * tooltip on the folder chip. The anchor is VIRTUAL, the trigger stretched up to the composer
 * card's top (`footerCardAnchorRect`), so `side="top"` hangs the card above the whole composer.
 */
export function FooterCardRoot({
  card,
  children,
}: {
  card: FooterCardState;
  children: React.ReactNode;
}) {
  return (
    <Popover open={card.open} onOpenChange={card.onOpenChange}>
      <PopoverAnchor virtualRef={card.anchor} />
      {children}
    </Popover>
  );
}

type TriggerProps = Omit<
  React.ButtonHTMLAttributes<HTMLButtonElement>,
  'type' | 'aria-haspopup' | 'aria-expanded' | 'aria-controls'
> & {
  card: FooterCardState;
  /** The card's surface id, for `aria-controls`. */
  cardId: string;
  'data-testid'?: string;
};

/**
 * A footer item's trigger: a real button on the line's item recipe (`.br-footline__item`), named
 * by its own `aria-label`, whose hover, focus and click are the card's (`footerCardTriggerProps`).
 * An `onClick` given here runs first; calling `event.preventDefault()` in it keeps the click from
 * pinning (the folder chip's Cmd or Ctrl-click, which opens the folder instead).
 */
export function FooterCardTrigger({
  card,
  cardId,
  className,
  onClick,
  children,
  ...rest
}: TriggerProps) {
  const trigger = footerCardTriggerProps(card, cardId);
  return (
    <button
      type="button"
      {...rest}
      {...trigger}
      className={cn('br-footline__item', className)}
      onClick={(event) => {
        onClick?.(event);
        if (event.defaultPrevented) return;
        trigger.onClick(event);
      }}
    >
      {children}
    </button>
  );
}

export interface FooterCardProps {
  card: FooterCardState;
  /** The surface id, for the trigger's `aria-controls`. */
  id: string;
  /** The card's accessible name (it is a `dialog`). */
  label: string;
  /** `start` for the folder on the line's left, `end` for the readouts on its right. */
  align: 'start' | 'end';
  testId?: string;
  children: React.ReactNode;
}

/**
 * The surface: above the anchor, which `useFooterCard` stretches from the trigger up to the
 * composer card's top, so the card hangs over the transcript and never over the draft or its
 * controls. Sections stack 12px apart. The room above caps the height (Radix's available height in `.br-footer-card`'s `max-height`,
 * which honours the collision padding), so a tall card in a short window scrolls inside itself.
 *
 * It looks the same whether a hover previews it or a click keeps it: the kept state is the
 * trigger's to show (`data-pinned`, the strong fill), as an open menu's is. The surface carries
 * `data-pinned` too, unstyled, so a test can read the state off either end.
 */
export function FooterCard({ card, id, label, align, testId, children }: FooterCardProps) {
  return (
    <PopoverContent
      id={id}
      ref={card.contentRef}
      side="top"
      align={align}
      collisionPadding={FOOTER_CARD_COLLISION_PADDING}
      aria-label={label}
      data-testid={testId}
      data-pinned={card.pinned ? 'true' : undefined}
      className="br-footer-card"
      {...card.contentHandlers}
    >
      {children}
    </PopoverContent>
  );
}

/** One topic of a card: its label, then what there is to read and do about it. */
export function FooterCardSection({
  label,
  labelId,
  children,
}: {
  label: string;
  /** The label's id, for a list in the section that is named by it. */
  labelId?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="br-footer-card__section">
      <p id={labelId} className="br-footer-card__label">
        {label}
      </p>
      {children}
    </section>
  );
}

/** A section's primary reading, in the default ink with tabular figures. */
export function FooterCardValue({
  className,
  children,
  ...rest
}: { children: React.ReactNode } & React.HTMLAttributes<HTMLParagraphElement>) {
  return (
    <p {...rest} className={cn('br-footer-card__value', className)}>
      {children}
    </p>
  );
}

/**
 * A folder, as a reading: a `FooterCardValue` like any other (the same 13/18, the same default
 * ink, the card's own face), written as the app writes a folder everywhere, `~` for the home
 * folder and absolute elsewhere (`tildePath(path, homeFolder())`, utils/folderPath.ts). The whole
 * location in one ink, never truncated: the card is the one place it is read in full.
 *
 * - Drawn by `PathSegments` (ui/path-segments.tsx), as the transcript's folder-access approval
 *   draws its folder: each name its own `<bdi>` with the separators outside it, in a
 *   left-to-right paragraph, so a right-to-left name cannot reorder the separators around it.
 * - It breaks only after a separator, and inside a name only when that name is wider than the
 *   card: each name and the separator after it are one atomic piece (`.br-path-piece` in
 *   `.br-footer-card__path`, pickers.css), so a hyphen or a space in a name that fits a line is no
 *   break point. A leading separator belongs to the first name, so a root never ends a line alone.
 * - `data-path` holds the absolute path it stands for; a selection copies what is drawn, `~` and
 *   all, with nothing added (the pieces and `<bdi>`s add no characters).
 * - A triple-click selects the path and the paragraph break after it (Chromium's paragraph
 *   selection runs to the start of the next block), so its copy read "~/Desktop/BioRouter\n\n",
 *   and pasted after `cd ` in a terminal it ran the line. A copy that starts in the path drops the
 *   line breaks at the selection's two ends (`copyWithoutEdgeBreaks`); one with none is left to
 *   the browser.
 */
export function FooterCardPath({ path }: { path: string }) {
  return (
    <FooterCardValue
      dir="ltr"
      data-path={path}
      className="br-footer-card__path"
      onCopy={copyWithoutEdgeBreaks}
    >
      <PathSegments path={tildePath(path, homeFolder())} />
    </FooterCardValue>
  );
}

/** Line breaks at either end of a selection's text: what a paragraph selection adds. */
const EDGE_BREAKS = /^[\r\n]+|[\r\n]+$/g;

/** Copy the selection without the line breaks a paragraph selection carries at its ends. */
function copyWithoutEdgeBreaks(event: React.ClipboardEvent<HTMLElement>) {
  const text = window.getSelection()?.toString() ?? '';
  const trimmed = text.replace(EDGE_BREAKS, '');
  if (trimmed === text || trimmed === '' || !event.clipboardData) return;
  event.preventDefault();
  event.clipboardData.setData('text/plain', trimmed);
}

/** A muted line under a value: what it is, in fewer words ("Read only · includes subfolders"). */
export function FooterCardDetail({ children }: { children: React.ReactNode }) {
  return <p className="br-footer-card__detail">{children}</p>;
}

/**
 * A muted explanatory sentence at the end of a section or of the card. `danger` is a failure the
 * person must see (with `role="alert"`); a transient state takes `role="status"`.
 */
export function FooterCardNote({
  tone = 'muted',
  children,
  ...rest
}: {
  tone?: 'muted' | 'danger';
  children: React.ReactNode;
} & React.HTMLAttributes<HTMLParagraphElement>) {
  return (
    <p {...rest} className="br-footer-card__note" data-tone={tone}>
      {children}
    </p>
  );
}

/** A list of rows (a folder per grant, a model per cost line). */
export function FooterCardList({
  labelledBy,
  children,
}: {
  /** The section label's id, so the list is named by what the person reads above it. */
  labelledBy?: string;
  children: React.ReactNode;
}) {
  return (
    <ul className="br-footer-card__list" aria-labelledby={labelledBy}>
      {children}
    </ul>
  );
}

/** One row: a value and its detail, and an optional action on the trailing edge. */
export function FooterCardRow({
  action,
  children,
}: {
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <li className="br-footer-card__row">
      <div className="br-footer-card__row-text">{children}</div>
      {action && <div className="br-footer-card__row-action">{action}</div>}
    </li>
  );
}

/** A row of the card's buttons, left-aligned, 8px apart. */
export function FooterCardActions({ children }: { children: React.ReactNode }) {
  return <div className="br-footer-card__actions">{children}</div>;
}

/**
 * Every button in a footer card: the compact secondary button, the same in all three. Its label
 * is set at the card's reading size (`.br-footer-card__action`, pickers.css), not the control
 * role's 14px, which made "Choose folder…" the largest text in the card.
 */
export function FooterCardAction({
  className,
  ...props
}: Omit<React.ComponentProps<typeof Button>, 'type' | 'variant' | 'size'>) {
  return (
    <Button
      type="button"
      variant="secondary"
      size="xs"
      className={cn('br-footer-card__action', className)}
      {...props}
    />
  );
}
