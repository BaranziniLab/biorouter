import './marketplace.css';
import * as React from 'react';
import { ModalShell } from '../ModalShell';
import { Input } from '../ui/input';
import { Skeleton } from '../ui/skeleton';
import { InfoTip } from '../ui/info-tip';
import { Search } from '../icons/app-icons';
import { cn } from '../../utils';
import { catalogFreshnessLine } from './registry';
import { MARKETPLACE_COPY } from './copy';

/**
 * The one shell both BAAM browse dialogs render through (implementation spec
 * §3.11, contract of WS-SKILLS).
 *
 * `BrowseExtensionsModal` and `BrowseSkillsModal` were near copies of one
 * hand-built dialog: a 720px width off the modal ladder, a header with a
 * paragraph of instructions, a raw `<input>`, rows drawn as boxed cards with an
 * icon tile and a tag row, and a footer that only held Close. This shell owns
 * everything the two share, once:
 *
 * - `ModalShell size="lg"` (640px), anchored at the top so the dialog grows
 *   downward and never re-centres under the pointer while a search narrows the
 *   list. The list keeps a floor height (`marketplace.css`) for the same reason.
 * - One visible line under the title, "From the Biorouter marketplace", plus the
 *   catalog's freshness when it is not live (§10.2: a last-good catalog is
 *   dated, not dismissed as offline). The dialog's own instructions sit in an
 *   InfoTip beside it, linked as a description so a screen reader hears them.
 * - The search field: the `Input` primitive with a leading glyph, placeholder
 *   "Search", named by `searchLabel`.
 * - The list states: skeleton rows while loading, one error line, one empty
 *   line. Nothing else is visible text.
 *
 * Rows are flat (`MarketplaceRow`): no icon tile, no tag row, the description
 * clamped to one line. Groups are `MarketplaceSection`s with a caps label and a
 * count.
 */
export type MarketplaceStatus = 'loading' | 'error' | 'ready';

export interface MarketplaceDialogProps {
  /** "Browse extensions" / "Browse skills". */
  title: string;
  /** The dialog's instructions, one or two short sentences, shown in the help tip. */
  help: string;
  /** From `loadRegistry`: a live catalog shows no freshness clause. */
  live: boolean;
  fetchedAt?: string;
  /** The search text, owned by the caller (it ranks the rows). */
  search: string;
  onSearchChange: (value: string) => void;
  /** Accessible name of the search field ("Search extensions"). */
  searchLabel: string;
  status: MarketplaceStatus;
  /** Ready, and nothing to show: renders `emptyText` in place of the list. */
  empty?: boolean;
  emptyText: string;
  /** A row under the search field: filter chips, "Select all". */
  toolbar?: React.ReactNode;
  /** The action row. Omit it for a dialog whose only action would be Close. */
  footer?: React.ReactNode;
  /** Work in flight: the dialog cannot be dismissed and loses its ×. */
  busy?: boolean;
  onClose: () => void;
  /** The list: `MarketplaceSection`s or bare `MarketplaceRow`s. */
  children?: React.ReactNode;
}

export function MarketplaceDialog({
  title,
  help,
  live,
  fetchedAt,
  search,
  onSearchChange,
  searchLabel,
  status,
  empty = false,
  emptyText,
  toolbar,
  footer,
  busy = false,
  onClose,
  children,
}: MarketplaceDialogProps) {
  const freshness = catalogFreshnessLine({ live, fetchedAt });

  return (
    <ModalShell
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
      size="lg"
      purpose={busy ? 'required' : 'info'}
      anchor="top"
      title={title}
      subtitle={
        <span className="inline-flex flex-wrap items-center gap-x-1">
          <span>
            {MARKETPLACE_COPY.subtitle}
            {freshness && <span className="text-text-subtle"> · {freshness}</span>}
          </span>
          <InfoTip label={title.toLowerCase()} help={help} />
        </span>
      }
      footer={footer}
      className="br-marketplace-dialog"
      bodyClassName="flex min-h-0 flex-1 flex-col"
    >
      <div className="flex flex-none flex-col gap-3 pb-3">
        <div className="relative">
          <Search
            aria-hidden="true"
            className="pointer-events-none absolute left-2 top-1/2 h-4 w-4 -translate-y-1/2 text-text-muted"
          />
          <Input
            type="text"
            autoFocus
            value={search}
            onChange={(event) => onSearchChange(event.target.value)}
            placeholder={MARKETPLACE_COPY.searchPlaceholder}
            aria-label={searchLabel}
            className="pl-8"
          />
        </div>
        {toolbar}
      </div>

      <div
        className={cn('br-marketplace-list', footer == null && 'br-marketplace-list--to-edge')}
        data-testid="marketplace-list"
      >
        {status === 'error' ? (
          <p role="alert" className="br-marketplace-state text-body text-text-danger">
            {MARKETPLACE_COPY.loadError}
          </p>
        ) : status === 'loading' ? (
          <div className="biorouter-list-shell">
            <span role="status" className="sr-only">
              {MARKETPLACE_COPY.loading}
            </span>
            <MarketplaceRowSkeleton />
            <MarketplaceRowSkeleton />
            <MarketplaceRowSkeleton />
          </div>
        ) : empty ? (
          <p className="br-marketplace-state text-body text-text-muted">{emptyText}</p>
        ) : (
          children
        )}
      </div>
    </ModalShell>
  );
}

export interface MarketplaceSectionProps {
  /** The group's name, sentence case; the caps style uppercases it. */
  label: string;
  count: number;
  children: React.ReactNode;
}

/** One group of rows under a caps label carrying its count ("Core skills 4"). */
export function MarketplaceSection({ label, count, children }: MarketplaceSectionProps) {
  return (
    <section className="br-marketplace-section">
      <h3 className="br-marketplace-section-label text-caps text-text-muted">
        {label} <span className="tabular-nums">{count}</span>
      </h3>
      <div className="biorouter-list-shell">{children}</div>
    </section>
  );
}

export interface MarketplaceRowProps {
  /**
   * `label` for a row whose whole surface toggles its `leading` checkbox (Browse
   * skills); `div` for a row whose action is a trailing button (Browse
   * extensions).
   */
  as?: 'div' | 'label';
  /** The entry's name. Rendered in a `span[data-marketplace-title]`. */
  title: string;
  /** Badges after the name: privacy, "Installed". */
  badges?: React.ReactNode;
  /** A short muted fact after the badges: organization · version, or the type. */
  meta?: React.ReactNode;
  /** The second line, clamped to one line. */
  description?: string;
  /** A control before the text (the selection checkbox). */
  leading?: React.ReactNode;
  /** Actions at the trailing edge. */
  trailing?: React.ReactNode;
  /** Dims nothing; only drops the pointer affordance of a `label` row. */
  disabled?: boolean;
  'data-testid'?: string;
}

export function MarketplaceRow({
  as = 'div',
  title,
  badges,
  meta,
  description,
  leading,
  trailing,
  disabled = false,
  'data-testid': testId,
}: MarketplaceRowProps) {
  const Comp = as;
  return (
    <Comp
      className={cn(
        'biorouter-list-row br-marketplace-row flex items-center gap-3 px-3 py-2',
        as === 'label' && !disabled && 'cursor-pointer'
      )}
      data-marketplace-row=""
      data-disabled={disabled ? '' : undefined}
      data-testid={testId}
    >
      {leading}
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="flex min-w-0 items-center gap-2">
          <span data-marketplace-title="" className="min-w-0 truncate text-label text-text-default">
            {title}
          </span>
          {badges}
          {meta != null && meta !== '' && (
            <span
              data-marketplace-meta=""
              className="min-w-0 truncate text-supporting text-text-muted"
            >
              {meta}
            </span>
          )}
        </span>
        {description && (
          <span className="block truncate text-supporting text-text-muted">{description}</span>
        )}
      </span>
      {trailing != null && <span className="flex shrink-0 items-center gap-2">{trailing}</span>}
    </Comp>
  );
}

function MarketplaceRowSkeleton() {
  return (
    <div className="biorouter-list-row flex items-center gap-3 px-3 py-2" aria-hidden>
      <div className="min-w-0 flex-1">
        <Skeleton className="h-4 w-40" />
        <Skeleton className="mt-1.5 h-3 w-72 max-w-full" />
      </div>
    </div>
  );
}

export default MarketplaceDialog;
