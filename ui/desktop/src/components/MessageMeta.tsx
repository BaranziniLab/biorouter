import './message-meta.css';
import * as React from 'react';
import { cn } from '../utils';

/** Shared message footer. Space stays reserved while hover/focus reveals it. */
interface MessageMetaProps {
  /** Preformatted timestamp. Omitted for a row that is only actions. */
  timestamp?: React.ReactNode;
  /**
   * Which edge the row hugs. `end` also reverses the order so the timestamp
   * stays nearest the edge the bubble is aligned to and the actions read
   * inward — the same relationship `start` has, mirrored.
   */
  align?: 'start' | 'end';
  className?: string;
  /** Actions — `MessageMetaAction`s, or anything of the same height. */
  children?: React.ReactNode;
}

export function MessageMeta({ timestamp, align = 'start', className, children }: MessageMetaProps) {
  const time = timestamp ? (
    <span className="text-supporting text-text-muted tabular-nums">{timestamp}</span>
  ) : null;

  return (
    <div
      data-message-meta={align}
      className={cn(
        'mt-2 flex min-h-5 flex-wrap items-center gap-x-3 gap-y-1',
        align === 'end' ? 'justify-end' : 'justify-start',
        className
      )}
    >
      {align === 'end' ? (
        <>
          {children}
          {time}
        </>
      ) : (
        <>
          {time}
          {children}
        </>
      )}
    </div>
  );
}

/**
 * One action inside a `MessageMeta` row — Copy, Diverge, Edit.
 *
 * The three were byte-similar 40-character class strings that had already
 * drifted (one carried `rounded`, one did not; one had a disabled state, two did
 * not). Here they are one recipe: `supporting` ink, a 14px glyph, muted at rest
 * and full ink on hover, and no motion at all — the row does not move, so
 * nothing has to animate out of the way.
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
