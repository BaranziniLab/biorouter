'use client';

import * as React from 'react';
import * as PopoverPrimitive from '@radix-ui/react-popover';

import { cn } from '../../utils';

export const Popover = PopoverPrimitive.Root;
export const PopoverTrigger = PopoverPrimitive.Trigger;
export const PopoverPortal = PopoverPrimitive.Portal;
/**
 * Positions a popover against an element that is not its trigger (spec 2.6, wave 0): a field
 * that opens a menu below itself, a row that a summary card hangs from. The anchor takes no part
 * in opening or closing; the caller owns `open`.
 */
export const PopoverAnchor = PopoverPrimitive.Anchor;

export const PopoverContent = React.forwardRef<
  React.ElementRef<typeof PopoverPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof PopoverPrimitive.Content>
>(({ className, align = 'center', sideOffset = 6, ...props }, ref) => (
  <PopoverPrimitive.Portal>
    <PopoverPrimitive.Content
      ref={ref}
      align={align}
      sideOffset={sideOffset}
      className={cn(
        // design.md §4.5: --radius-container (12px), 4px padding, 6px trigger offset.
        // `.biorouter-popover-surface` supplies the border + shadow ONLY (no radius,
        // no background) — those two live here so every popover shares one geometry.
        //
        // Z — deliberately --z-modal-dropdown (500), not --z-dropdown (200):
        // this primitive is portalled to <body>, so it becomes a SIBLING of any
        // Radix dialog content (--z-modal, 400) it is rendered inside. It has no
        // way to know its host, and a real call site nests it in a modal:
        // WorkflowResourcePicker -> WorkflowFormFields -> CreateWorkflowFromSessionModal's
        // DialogContent. At 200 that picker would paint under the dialog it belongs to.
        'biorouter-popover-surface z-[var(--z-modal-dropdown)] w-60 rounded-container bg-background-default p-1 ',
        'data-[state=open]:animate-in data-[state=closed]:animate-out',
        'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
        'data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95',
        'data-[state=open]:duration-[var(--motion-base)] data-[state=closed]:duration-[var(--motion-fast)]',
        className
      )}
      {...props}
    />
  </PopoverPrimitive.Portal>
));
PopoverContent.displayName = 'PopoverContent';
