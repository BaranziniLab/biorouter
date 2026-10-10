'use client';

import * as React from 'react';
import * as ContextMenuPrimitive from '@radix-ui/react-context-menu';

import { cn } from '../../utils';
import {
  MENU_EASE_CLASS_NAME,
  MENU_GROUP_LABEL_CLASS_NAME,
  MENU_ROW_CLASS_NAME,
  MENU_SURFACE_CLASS_NAME,
} from './dropdown-menu';

/**
 * The right-click menu, drawn from the SAME surface and row tokens as
 * `dropdown-menu.tsx`.
 *
 * ⚠ **The row string is imported, not restated.** `MENU_ROW_CLASS_NAME` is
 * exported for exactly this: §4.5's menu row is one 32px/12px/`text-secondary`
 * rule, and a second menu that spelled its own padding and type size would
 * reintroduce the per-call-site drift that export exists to stop. A user who
 * right-clicks a chat row and a user who opens the same actions from the `⋯`
 * overflow must see the same menu, or the two entry points read as two
 * different features.
 *
 * Z is `--z-modal-dropdown` (500) for the reason the dropdown gives: the content
 * is portalled to `<body>`, making it a sibling of any dialog content it was
 * rendered inside, with no way to detect that host.
 *
 * `@radix-ui/react-context-menu` is imported directly, the way
 * `dropdown-menu.tsx` imports `@radix-ui/react-dropdown-menu` — both arrive
 * with `@radix-ui/themes` rather than as direct dependencies, so this adds no
 * package.
 */
function ContextMenu({ ...props }: React.ComponentProps<typeof ContextMenuPrimitive.Root>) {
  return <ContextMenuPrimitive.Root data-slot="context-menu" {...props} />;
}

/**
 * The right-click target. Use `asChild` so the menu attaches to the row the
 * surface already renders instead of wrapping it in a box that changes layout.
 *
 * **This is also the keyboard trigger, and deliberately not a second gesture.**
 * The Menu key and Shift+F10 both dispatch a `contextmenu` event on the focused
 * element in Chromium, so a keyboard user reaches this menu with the same
 * binding every other application uses. Inventing an app-specific shortcut
 * would give the same actions two vocabularies.
 */
const ContextMenuTrigger = ContextMenuPrimitive.Trigger;

function ContextMenuContent({
  className,
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.Content>) {
  return (
    <ContextMenuPrimitive.Portal>
      <ContextMenuPrimitive.Content
        data-slot="context-menu-content"
        // ⚠ `no-drag`, and it is load-bearing on exactly one of the three
        // surfaces: the tab strip sits in the titlebar band, where App.tsx
        // paints a 32px `-webkit-app-region: drag` rect. This menu opens AT THE
        // CURSOR rather than anchored below a trigger, so on a tab its top edge
        // lands inside that rect — and Electron folds app-region rects in DOM
        // order, so an earlier `drag` rect eats clicks on a higher-z control
        // whatever the z-index says (issue #74). The portal appends here after
        // App's tree, so this `no-drag` folds later and wins. The overflow
        // dropdown in the same band never needed it because it opens downward
        // from its trigger.
        className={cn(
          'no-drag',
          MENU_SURFACE_CLASS_NAME,
          'max-h-(--radix-context-menu-content-available-height) overflow-x-hidden overflow-y-auto',
          // The dropdown's ease, so a menu opened by right-click and the same menu opened from
          // `⋯` move alike (it used to fall back to the browser's `ease`).
          MENU_EASE_CLASS_NAME,
          className
        )}
        {...props}
      />
    </ContextMenuPrimitive.Portal>
  );
}

function ContextMenuItem({
  className,
  variant = 'default',
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.Item> & {
  variant?: 'default' | 'destructive';
}) {
  return (
    <ContextMenuPrimitive.Item
      data-slot="context-menu-item"
      data-variant={variant}
      className={cn(
        MENU_ROW_CLASS_NAME,
        // The dropdown's destructive treatment, word for word: danger ink, a 10% (dark 20%)
        // danger wash on highlight. One destructive look whichever way the menu was opened.
        "data-[variant=destructive]:text-text-danger data-[variant=destructive]:focus:bg-background-danger/10 dark:data-[variant=destructive]:focus:bg-background-danger/20 data-[variant=destructive]:focus:text-text-danger data-[variant=destructive]:*:[svg]:!text-text-danger [&_svg:not([class*='text-'])]:text-text-muted",
        className
      )}
      {...props}
    />
  );
}

function ContextMenuSeparator({
  className,
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.Separator>) {
  return (
    <ContextMenuPrimitive.Separator
      data-slot="context-menu-separator"
      className={cn('bg-border-subtle -mx-1 my-1 h-px', className)}
      {...props}
    />
  );
}

function ContextMenuLabel({
  className,
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.Label>) {
  return (
    <ContextMenuPrimitive.Label
      data-slot="context-menu-label"
      className={cn(MENU_GROUP_LABEL_CLASS_NAME, className)}
      {...props}
    />
  );
}

export {
  ContextMenu,
  ContextMenuTrigger,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuLabel,
};
