'use client';

import * as React from 'react';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import { XIcon } from '../icons/app-icons';

import { cn } from '../../utils';
import { installDialogTabRepair } from './dialogTabRepair';

function Dialog({ ...props }: React.ComponentProps<typeof DialogPrimitive.Root>) {
  return <DialogPrimitive.Root data-slot="dialog" {...props} />;
}

function DialogTrigger({ ...props }: React.ComponentProps<typeof DialogPrimitive.Trigger>) {
  return <DialogPrimitive.Trigger data-slot="dialog-trigger" {...props} />;
}

function DialogPortal({ ...props }: React.ComponentProps<typeof DialogPrimitive.Portal>) {
  return <DialogPrimitive.Portal data-slot="dialog-portal" {...props} />;
}

function DialogClose({ ...props }: React.ComponentProps<typeof DialogPrimitive.Close>) {
  return <DialogPrimitive.Close data-slot="dialog-close" {...props} />;
}

const DialogOverlay = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Overlay>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Overlay>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Overlay
    ref={ref}
    data-slot="dialog-overlay"
    className={cn(
      'biorouter-modal-overlay data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 fixed inset-0 z-[var(--z-overlay)]',
      className
    )}
    {...props}
  />
));
DialogOverlay.displayName = DialogPrimitive.Overlay.displayName;

function DialogContent({
  className,
  children,
  dismissible = true,
  showCloseButton = dismissible,
  onEscapeKeyDown,
  onPointerDownOutside,
  ref,
  ...props
}: React.ComponentProps<typeof DialogPrimitive.Content> & {
  dismissible?: boolean;
  showCloseButton?: boolean;
}) {
  // See `dialogTabRepair`: a `Select` unmounting its live region on blur makes
  // Radix's focus scope swallow the Tab that was moving off it, which left the
  // New schedule dialog's Cancel and "Create schedule" keyboard-unreachable.
  // The repair belongs here, on the one primitive every modal composes, because
  // the trigger is any `Select` (or anything else that mutates the dialog's DOM
  // while blurring) rather than anything the schedule dialog does.
  const attachRepair = React.useCallback(
    (node: HTMLDivElement | null) => {
      const forward = (value: HTMLDivElement | null) => {
        if (typeof ref === 'function') ref(value);
        else if (ref) (ref as React.RefObject<HTMLDivElement | null>).current = value;
      };
      forward(node);
      if (!node) return;
      const teardown = installDialogTabRepair(node);
      // React 19 calls a ref callback's cleanup INSTEAD of re-invoking it with
      // `null`, so the forwarded ref has to be cleared here or a caller holding
      // one would keep a detached node.
      return () => {
        teardown();
        forward(null);
      };
    },
    [ref]
  );

  return (
    <DialogPortal data-slot="dialog-portal">
      <DialogOverlay />
      <DialogPrimitive.Content
        ref={attachRepair}
        // Radix does not set this itself — it isolates the rest of the page with
        // `aria-hidden` instead — so a screen reader was told this was a dialog
        // but never that it was a modal one.
        aria-modal="true"
        data-slot="dialog-content"
        className={cn(
          // No radius utility here on purpose: `.biorouter-modal-surface` (main.css,
          // unlayered, so it outranks Tailwind's utilities layer) already owns the
          // 16px dialog corner. A `rounded-*` here could only duplicate it or lie.
          'biorouter-modal-surface bg-background-default data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 fixed top-[50%] left-[50%] z-[var(--z-modal)] grid w-full max-w-[calc(100%-2rem)] translate-x-[-50%] translate-y-[-50%] gap-4 p-6 duration-[var(--motion-base)] data-[state=closed]:duration-[var(--motion-fast)] sm:max-w-lg',
          className
        )}
        onEscapeKeyDown={(event) => {
          onEscapeKeyDown?.(event);
          if (!dismissible) event.preventDefault();
        }}
        onPointerDownOutside={(event) => {
          onPointerDownOutside?.(event);
          if (!dismissible) event.preventDefault();
        }}
        {...props}
      >
        {children}
        {showCloseButton && (
          <DialogPrimitive.Close
            disabled={!dismissible}
            className="p-2 flex items-center justify-center hover:bg-overlay-hover rounded-element data-[state=open]:bg-overlay-hover transition-[background-color,color,opacity] data-[state=open]:text-text-muted absolute top-4 right-4 opacity-70 hover:opacity-100 disabled:pointer-events-none [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4"
          >
            <XIcon />
            <span className="sr-only">Close</span>
          </DialogPrimitive.Close>
        )}
      </DialogPrimitive.Content>
    </DialogPortal>
  );
}

function DialogHeader({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="dialog-header"
      className={cn('flex flex-col gap-1 mb-2 pr-8 text-center sm:text-left', className)}
      {...props}
    />
  );
}

function DialogFooter({ className, ...props }: React.ComponentProps<'div'>) {
  return (
    <div
      data-slot="dialog-footer"
      className={cn('flex flex-col-reverse gap-2 sm:flex-row sm:justify-end', className)}
      {...props}
    />
  );
}

function DialogTitle({ className, ...props }: React.ComponentProps<typeof DialogPrimitive.Title>) {
  return (
    <DialogPrimitive.Title
      data-slot="dialog-title"
      className={cn('text-subheading', className)}
      {...props}
    />
  );
}

function DialogDescription({
  className,
  ...props
}: React.ComponentProps<typeof DialogPrimitive.Description>) {
  return (
    <DialogPrimitive.Description
      data-slot="dialog-description"
      className={cn('text-text-muted text-body', className)}
      {...props}
    />
  );
}

export {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogOverlay,
  DialogPortal,
  DialogTitle,
  DialogTrigger,
};
