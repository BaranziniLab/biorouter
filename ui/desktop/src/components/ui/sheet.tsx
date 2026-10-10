'use client';

import * as React from 'react';
import * as SheetPrimitive from '@radix-ui/react-dialog';
import { XIcon } from '../icons/app-icons';

import { cn } from '../../utils';

function Sheet({ ...props }: React.ComponentProps<typeof SheetPrimitive.Root>) {
  return <SheetPrimitive.Root data-slot="sheet" {...props} />;
}

function SheetTrigger({ ...props }: React.ComponentProps<typeof SheetPrimitive.Trigger>) {
  return <SheetPrimitive.Trigger data-slot="sheet-trigger" {...props} />;
}

function SheetClose({ ...props }: React.ComponentProps<typeof SheetPrimitive.Close>) {
  return <SheetPrimitive.Close data-slot="sheet-close" {...props} />;
}

function SheetPortal({ ...props }: React.ComponentProps<typeof SheetPrimitive.Portal>) {
  return <SheetPrimitive.Portal data-slot="sheet-portal" {...props} />;
}

const SheetOverlay = React.forwardRef<
  React.ElementRef<typeof SheetPrimitive.Overlay>,
  React.ComponentPropsWithoutRef<typeof SheetPrimitive.Overlay>
>(({ className, ...props }, ref) => (
  <SheetPrimitive.Overlay
    ref={ref}
    data-slot="sheet-overlay"
    className={cn(
      'biorouter-modal-overlay data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 fixed inset-0 z-[var(--z-overlay)]',
      className
    )}
    {...props}
  />
));
SheetOverlay.displayName = SheetPrimitive.Overlay.displayName;

/**
 * A drawer, and the one rule its callers keep getting wrong.
 *
 * ⚠ **Every `SheetContent` must settle its description, one way or the other.**
 * Radix writes an `aria-describedby` onto this element unconditionally, pointing
 * at the id a `SheetDescription` would claim; if no description renders, the
 * attribute is left dangling at nothing and Radix logs *"Missing `Description`
 * or `aria-describedby={undefined}` for {DialogContent}"* on every open. So a
 * call site does one of exactly two things:
 *
 * - **It has a description** — render `SheetDescription` (see `ui/sidebar.tsx`).
 *   Radix links it automatically; pass nothing here.
 * - **It genuinely has none** — pass `aria-describedby={undefined}`, Radix's own
 *   opt-out. Both Knowledge drawers do: their headers are a bare title, and a
 *   description written only for the accessibility tree would say something the
 *   screen shows nowhere.
 *
 * The choice cannot be defaulted here. Forcing the opt-out would silently unlink
 * `ui/sidebar.tsx`'s real description — a worse bug than the warning, and a
 * silent one — so it stays at the call site, where the answer is known.
 * `ModalShell` encodes the same policy for centred modals, which is why every
 * modal that renders through it (`KBManagerDialog` among them) is already clean.
 */
function SheetContent({
  className,
  children,
  side = 'right',
  ...props
}: React.ComponentProps<typeof SheetPrimitive.Content> & {
  side?: 'top' | 'right' | 'bottom' | 'left';
}) {
  return (
    <SheetPortal>
      <SheetOverlay />
      <SheetPrimitive.Content
        data-slot="sheet-content"
        className={cn(
          // Motion (spec 2.6): `.br-sheet-motion` in main.css, in over `--dur-med`, out over
          // `--dur-fast`, `--ease-out`; the slide utilities below only set the direction.
          'biorouter-modal-surface br-sheet-motion bg-background-default shadow-modal data-[state=open]:animate-in data-[state=closed]:animate-out fixed z-[var(--z-modal)] flex flex-col gap-4',
          side === 'right' &&
            'data-[state=closed]:slide-out-to-right data-[state=open]:slide-in-from-right inset-y-0 right-0 h-full w-3/4 border-l sm:max-w-sm',
          side === 'left' &&
            'data-[state=closed]:slide-out-to-left data-[state=open]:slide-in-from-left inset-y-0 left-0 h-full w-3/4 border-r sm:max-w-sm',
          side === 'top' &&
            'data-[state=closed]:slide-out-to-top data-[state=open]:slide-in-from-top inset-x-0 top-0 h-auto border-b',
          side === 'bottom' &&
            'data-[state=closed]:slide-out-to-bottom data-[state=open]:slide-in-from-bottom inset-x-0 bottom-0 h-auto border-t',
          className
        )}
        {...props}
      >
        {children}
        <SheetPrimitive.Close className="data-[state=open]:bg-overlay-hover absolute top-4 right-4 flex h-8 w-8 items-center justify-center rounded-element text-text-muted opacity-70 transition-[opacity,background-color] hover:bg-overlay-hover hover:opacity-100 disabled:pointer-events-none">
          <XIcon className="size-4" />
          <span className="sr-only">Close</span>
        </SheetPrimitive.Close>
      </SheetPrimitive.Content>
    </SheetPortal>
  );
}

function SheetHeader({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="sheet-header"
      className={cn('flex flex-col gap-1.5 p-4 pr-14', className)}
      {...props}
    />
  );
}

function SheetFooter({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="sheet-footer"
      className={cn('mt-auto flex flex-col gap-2 p-4', className)}
      {...props}
    />
  );
}

function SheetTitle({ className, ...props }: React.ComponentProps<typeof SheetPrimitive.Title>) {
  return (
    <SheetPrimitive.Title
      data-slot="sheet-title"
      className={cn('text-text-default text-subheading', className)}
      {...props}
    />
  );
}

function SheetDescription({
  className,
  ...props
}: React.ComponentProps<typeof SheetPrimitive.Description>) {
  return (
    <SheetPrimitive.Description
      data-slot="sheet-description"
      className={cn('text-text-muted text-body', className)}
      {...props}
    />
  );
}

export {
  Sheet,
  SheetTrigger,
  SheetClose,
  SheetContent,
  SheetHeader,
  SheetFooter,
  SheetTitle,
  SheetDescription,
};
