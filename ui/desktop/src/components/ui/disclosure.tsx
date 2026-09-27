import * as React from 'react';

import { cn } from '../../utils';
import { ChevronRight } from '../icons/app-icons';
import { Button } from './button';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from './collapsible';

export interface DisclosureProps {
  /** The trigger's words, and its accessible name. */
  label?: string;
  /**
   * A muted line shown beside the trigger while it is CLOSED, stating what the
   * hidden fields default to ("Port 22 · your SSH settings"). It is also the
   * trigger's accessible description, so a screen reader hears the defaults
   * without opening anything. Hidden once open: the fields say it themselves.
   */
  summary?: React.ReactNode;
  open?: boolean;
  /** Start open — for a saved record that already uses a field inside. */
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  children: React.ReactNode;
  /** Layout only, on the root. */
  className?: string;
}

/**
 * The one progressive-disclosure control: "Advanced", "How do I verify it?",
 * "Trouble signing in?". One level only — a Disclosure inside a Disclosure is a
 * form that needs a different design, not a deeper tree.
 *
 * - The trigger is a ghost `sm` Button in muted ink with a `ChevronRight` that
 *   turns 90° when open (`--dur-fast-max` in, `--dur-fast` out).
 * - The body is Radix `Collapsible` content, which is **unmounted when closed**:
 *   a closed Disclosure contributes nothing to the tab order, the accessibility
 *   tree or a form submission. A caller that needs to reveal an invalid hidden
 *   field controls `open` and focuses the field after opening.
 * - The height animation is authored CSS (`.biorouter-disclosure-panel` in
 *   `main.css`): Tailwind's `collapsible-down/up` keyframes are not emitted by
 *   this build, and a newly written `animate-*` utility can silently fail to
 *   generate under `BIOROUTER_NO_HMR`. Radix's presence waits for the closing
 *   animation before it unmounts, and reduced motion makes both instant.
 */
export function Disclosure({
  label = 'Advanced',
  summary,
  open,
  defaultOpen,
  onOpenChange,
  children,
  className,
}: DisclosureProps) {
  const summaryId = React.useId();
  // Controlled or not, the summary follows the state Radix is actually in.
  const [uncontrolledOpen, setUncontrolledOpen] = React.useState(defaultOpen ?? false);
  const isOpen = open ?? uncontrolledOpen;
  const showSummary = !isOpen && summary !== undefined && summary !== null && summary !== '';

  const handleOpenChange = (next: boolean) => {
    if (open === undefined) setUncontrolledOpen(next);
    onOpenChange?.(next);
  };

  return (
    <Collapsible
      open={isOpen}
      onOpenChange={handleOpenChange}
      className={cn('biorouter-disclosure', className)}
    >
      <div className="biorouter-disclosure-head">
        <CollapsibleTrigger asChild>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="biorouter-disclosure-trigger text-text-muted"
            aria-describedby={showSummary ? summaryId : undefined}
          >
            <ChevronRight aria-hidden className="biorouter-disclosure-chevron" />
            {label}
          </Button>
        </CollapsibleTrigger>
        {showSummary ? (
          <span id={summaryId} className="biorouter-disclosure-summary">
            {summary}
          </span>
        ) : null}
      </div>
      <CollapsibleContent className="biorouter-disclosure-panel">
        <div className="biorouter-disclosure-body">{children}</div>
      </CollapsibleContent>
    </Collapsible>
  );
}
