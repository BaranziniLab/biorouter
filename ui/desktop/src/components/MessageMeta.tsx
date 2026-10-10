import './message-meta.css';
import * as React from 'react';
import { cn } from '../utils';

/**
 * A message's floating action cluster (Crew's row actions; `message-meta.css`).
 *
 * It holds 24px icon-only actions (`TranscriptIconButton`) and the message's
 * time as a 12px muted label. It is absolutely positioned at the top right of
 * the nearest `.br-message-actions-anchor` (or scope), so nothing in the flow is
 * reserved for it, and it is revealed by hovering or focusing anything inside
 * the nearest `.br-message-actions-scope`.
 *
 * `data-message-meta` stays: the user-message layout e2e measures the bubble's
 * right edge against this box (`tests/e2e/user-message-layout.spec.ts`).
 */
interface MessageActionsProps {
  /** Preformatted time, shown first. Omitted for a cluster that is only actions. */
  timestamp?: React.ReactNode;
  /** The edge the message hugs; the cluster is always at its right. */
  align?: 'start' | 'end';
  className?: string;
  /** The actions: `TranscriptIconButton`s or a menu trigger built on one. */
  children?: React.ReactNode;
}

export function MessageActions({
  timestamp,
  align = 'start',
  className,
  children,
}: MessageActionsProps) {
  return (
    <div data-message-meta={align} className={cn('br-message-actions', className)}>
      {timestamp ? (
        <span className="br-message-actions-time text-supporting text-text-muted tabular-nums">
          {timestamp}
        </span>
      ) : null}
      {children}
    </div>
  );
}

/**
 * An action that stays in the flow under a message because it is about the
 * message's content, not an action on it: "Show more" on a long message, "Send
 * again" on a steer nobody read. A 12px label with a 14px glyph, muted at rest
 * and full ink on hover; nothing moves.
 */
interface MessageMetaActionProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  icon: React.ReactNode;
  children: React.ReactNode;
}

export function MessageMetaAction({ icon, children, className, ...props }: MessageMetaActionProps) {
  return (
    <button
      type="button"
      {...props}
      className={cn(
        'flex items-center gap-1 rounded-inner text-supporting text-text-muted',
        'transition-colors hover:text-text-default hover:cursor-pointer',
        'disabled:cursor-default disabled:opacity-50 disabled:hover:text-text-muted',
        '[&_svg]:size-3.5 [&_svg]:shrink-0',
        className
      )}
    >
      {icon}
      <span>{children}</span>
    </button>
  );
}
