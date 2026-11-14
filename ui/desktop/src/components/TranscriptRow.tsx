import './transcript-row.css';
import * as React from 'react';
import { cn } from '../utils';
import { ChevronRight } from './icons/app-icons';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from './ui/collapsible';

/** Any 16px glyph component: a tool glyph, `Brain`, `Check`, `AlertTriangle`. */
export type TranscriptRowIcon = React.ComponentType<{ className?: string }>;

export interface TranscriptRowProps {
  /** The leading glyph, drawn at 16px. For a tool call: `toolGlyphFor(name, args).Icon`. */
  icon: TranscriptRowIcon;
  /** The glyph's kind, stamped as `data-tool-glyph` so tests and probes can read it. */
  glyph?: string;
  /** The verb and summary ("Ran ls -la", "Thinking"). Truncates on one line. */
  label: React.ReactNode;
  /**
   * Short muted text after the label that never truncates ("· 8s"). It sits
   * inside the trigger, so it must not be interactive.
   */
  meta?: React.ReactNode;
  /**
   * Content after the row, outside the trigger, so it may hold a control (an
   * InfoTip explaining the meta). Never inside the button.
   */
  trailing?: React.ReactNode;
  /**
   * `inline` (default) sets `trailing` after the head on the row's line. `below` gives it a
   * line of its own under the head, starting at the label's left edge, so a trailing part that
   * holds several controls never squeezes the label (a `dictionary__add_terms` row's terms,
   * each beside its own Keep and Undo).
   */
  trailingPlacement?: 'inline' | 'below';
  /** Work in progress: the label breathes until the row is hovered or focused. */
  running?: boolean;
  /**
   * `danger` draws the line in danger ink: a call that failed, or a row that
   * failed to render. Only the row's own line; rows nested in its body keep
   * their own tone.
   */
  tone?: 'muted' | 'danger';
  /** A screen-reader status beside the glyph ("Tool status: success"). */
  statusLabel?: string;
  /**
   * The disclosure body. Without children the row is a static line: no
   * chevron, no hover, nothing to focus.
   */
  children?: React.ReactNode;
  /**
   * `well` (default) puts the body in one indented well on `--background-well`.
   * `plain` only indents it 24px, for prose such as the thinking text.
   */
  body?: 'well' | 'plain';
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** Layout only, on the root. */
  className?: string;
  /** Extra class on the trigger (a test hook such as `br-tool-disclosure`). */
  triggerClassName?: string;
  'data-testid'?: string;
}

/**
 * The one transcript line (D-17: a tool call is a line, not a card).
 *
 * `[glyph 16] Verb summary · meta ›` at 28px in `text-secondary` muted ink,
 * opening into one well indented 24px so the well's left edge is the label's.
 * Tool calls, the thinking disclosure, a parked tool call and a resolved card
 * all draw through it, so the transcript has one disclosure dialect.
 *
 * - The trigger bleeds 8px left for its hover wash (`--overlay-hover`, radius
 *   8); the glyph stays on the column edge.
 * - The body is Radix `Collapsible` content on the shared
 *   `.biorouter-disclosure-panel` height animation (300ms open, 125ms close),
 *   and it is unmounted while closed.
 * - Focus follows D-15 for rows: `--background-focus` plus an inset 2px edge.
 */
export function TranscriptRow({
  icon: Icon,
  glyph,
  label,
  meta,
  trailing,
  trailingPlacement = 'inline',
  running = false,
  tone = 'muted',
  statusLabel,
  children,
  body = 'well',
  open,
  defaultOpen,
  onOpenChange,
  className,
  triggerClassName,
  'data-testid': testId,
}: TranscriptRowProps) {
  const [uncontrolledOpen, setUncontrolledOpen] = React.useState(defaultOpen ?? false);
  const isOpen = open ?? uncontrolledOpen;
  const expandable = children !== undefined && children !== null && children !== false;

  const handleOpenChange = (next: boolean) => {
    if (open === undefined) setUncontrolledOpen(next);
    onOpenChange?.(next);
  };

  const head = (
    <>
      <span className="br-transcript-row-glyph" data-tool-glyph={glyph} aria-hidden="true">
        <Icon />
      </span>
      {statusLabel ? <span role="img" className="sr-only" aria-label={statusLabel} /> : null}
      <span className="br-transcript-row-label">{label}</span>
      {meta !== undefined && meta !== null && meta !== false ? (
        <>
          {/* A space for the spoken name ("Thinking · 9s", not "Thinking· 9s").
              Between two flex items it takes no room; the gap does the spacing. */}{' '}
          <span className="br-transcript-row-meta">{meta}</span>
        </>
      ) : null}
    </>
  );

  // Below the head, the trailing part is still inside the row's line, which wraps: the line is
  // the element `message-meta.css` ends short under a message's hover actions, so the trailing
  // line ends short with it.
  const below = trailingPlacement === 'below' && trailing !== undefined && trailing !== null;
  const lineProps = { 'data-trailing': below ? 'below' : undefined };
  const trailingPart = below ? (
    <div className="br-transcript-row-trailing">{trailing}</div>
  ) : (
    trailing
  );

  const rootProps = {
    'data-testid': testId,
    'data-tone': tone,
    'data-running': running ? '' : undefined,
  };

  if (!expandable) {
    return (
      <div {...rootProps} className={cn('br-transcript-row', className)}>
        <div className="br-transcript-row-line" {...lineProps}>
          <div className={cn('br-transcript-row-head', triggerClassName)}>{head}</div>
          {trailingPart}
        </div>
      </div>
    );
  }

  return (
    <Collapsible
      {...rootProps}
      open={isOpen}
      onOpenChange={handleOpenChange}
      className={cn('br-transcript-row', className)}
    >
      <div className="br-transcript-row-line" {...lineProps}>
        <CollapsibleTrigger asChild>
          <button type="button" className={cn('br-transcript-row-head', triggerClassName)}>
            {head}
            <ChevronRight aria-hidden="true" className="br-transcript-row-chevron" />
          </button>
        </CollapsibleTrigger>
        {trailingPart}
      </div>
      <CollapsibleContent className="biorouter-disclosure-panel">
        <div className="br-transcript-row-body">
          {body === 'well' ? <div className="br-transcript-row-well">{children}</div> : children}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}

/**
 * One labelled part of a well ("Input", "Output", "Logs"). Sections sit 8px
 * apart with no rules; the label is `text-supporting` muted.
 */
export function TranscriptRowSection({
  label,
  children,
}: {
  label: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="br-transcript-row-section">
      <div className="br-transcript-row-section-label">{label}</div>
      {children}
    </section>
  );
}
