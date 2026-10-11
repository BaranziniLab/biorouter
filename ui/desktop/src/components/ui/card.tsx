import * as React from 'react';

import { cn } from '../../utils';

/**
 * A card is a subtle solid fill, not an outline (owner message 13, R-27, design.md 9.11):
 * `--background-fill` with the 1px border kept but transparent (no geometry moves, and forced
 * colours still draw the edge in the system ink). `.br-fill-surface` steps the controls inside it
 * back to the page ground, so a field or a secondary button never sits fill on fill.
 */
function Card({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="card"
      className={cn(
        'br-fill-surface bg-background-fill text-text-default flex flex-col gap-4 rounded-container border border-transparent py-4',
        className
      )}
      {...props}
    />
  );
}

function CardHeader({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="card-header"
      className={cn(
        '@container/card-header grid auto-rows-min grid-rows-[auto_auto] items-start px-4 has-data-[slot=card-action]:grid-cols-[1fr_auto] [.border-b]:pb-4',
        className
      )}
      {...props}
    />
  );
}

function CardTitle({ className, ...props }: React.ComponentProps<'div'>) {
  return <div data-slot="card-title" className={cn('leading-none', className)} {...props} />;
}

function CardDescription({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="card-description"
      className={cn('text-text-muted text-body', className)}
      {...props}
    />
  );
}

function CardAction({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="card-action"
      className={cn('col-start-2 row-span-2 row-start-1 self-start justify-self-end', className)}
      {...props}
    />
  );
}

function CardContent({ className, ...props }: React.ComponentProps<'div'>) {
  return <div data-slot="card-content" className={cn('px-4', className)} {...props} />;
}

function CardFooter({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="card-footer"
      className={cn('flex items-center px-4 [.border-t]:pt-4', className)}
      {...props}
    />
  );
}

export { Card, CardHeader, CardFooter, CardTitle, CardAction, CardDescription, CardContent };
