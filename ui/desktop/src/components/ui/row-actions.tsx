'use client';

import * as React from 'react';
import { Slot } from '@radix-ui/react-slot';

import { cn } from '../../utils';
import { MoreHorizontal } from '../icons/app-icons';
import { Button } from './button';
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from './context-menu';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from './dropdown-menu';
import { isContextMenuKey, openContextMenuFromKeyboard } from './keyboardContextMenu';
import { Tooltip, TooltipContent, TooltipTrigger } from './Tooltip';
import { uiCopy } from './copy';

/**
 * RowActions and RowContextMenu: actions appear when wanted (spec 2.6, principle 6; Crew's
 * `.crew-member-actions` / `[data-row-action]` recipe).
 *
 * ```tsx
 * <RowContextMenu items={items}>
 *   <div className="biorouter-list-row">
 *     …
 *     <RowActions primary={<IconAction icon={Play} label="Run" onSelect={run} />} menu={items}
 *                 meta={<time>2h</time>} />
 *   </div>
 * </RowContextMenu>
 * ```
 *
 * - The cluster (`.br-row-actions`) goes from opacity 0 to 1 over 95ms when its row is hovered,
 *   holds focus, or has its menu open, and it is always shown under `(hover: none)`. Only the
 *   opacity changes, so every action stays in the tab order.
 * - One primary icon action at most, then a `⋯` button (ghost round, Tooltip "More actions")
 *   that opens the items as a DropdownMenu.
 * - `RowContextMenu` makes the same items a ContextMenu on the row: right-click, Shift+F10 or
 *   the ContextMenu key (macOS Chromium never turns the keys into `contextmenu`, so the row
 *   dispatches it; see `keyboardContextMenu.ts`).
 * - `meta` (a relative time, a count) shows at rest and gives way to the actions on hover or
 *   focus.
 *
 * Chat-row and view menus are text-only (Crew's ChannelRow menus); other menus may keep a 16px
 * icon per item.
 */

export type RowActionIcon = React.ComponentType<{ className?: string; 'aria-hidden'?: boolean }>;

export type RowActionItem =
  | {
      kind?: 'item';
      label: string;
      onSelect: () => void;
      icon?: RowActionIcon;
      /** Danger ink and wash: a destructive action. */
      destructive?: boolean;
      disabled?: boolean;
      /** A shortcut hint at the trailing edge (`text-supporting` muted). */
      shortcut?: string;
      testId?: string;
    }
  | { kind: 'separator' }
  | { kind: 'label'; label: string };

export type RowActionSize = 'xs' | 'sm' | 'default';

export interface IconActionProps {
  icon: RowActionIcon;
  /** The action's name: the button's accessible name, and its tooltip unless `tooltip` is set. */
  label: string;
  /** A shorter tooltip than the name ("Pause" for "Pause nightly-cohort"). */
  tooltip?: string;
  onSelect: () => void;
  disabled?: boolean;
  /** 24, 28 or 32px; 32 (`default`) in 40px content rows. */
  size?: RowActionSize;
  testId?: string;
  className?: string;
}

/** A ghost, round icon button named by its tooltip: the row's one primary action. */
export const IconAction = React.forwardRef<HTMLButtonElement, IconActionProps>(function IconAction(
  { icon: Icon, label, tooltip, onSelect, disabled, size = 'default', testId, className },
  ref
) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          ref={ref}
          type="button"
          variant="ghost"
          shape="round"
          size={size}
          aria-label={label}
          disabled={disabled}
          data-testid={testId}
          data-row-action=""
          className={className}
          onClick={(event) => {
            // The row may navigate on click; the action is its own control.
            event.stopPropagation();
            onSelect();
          }}
        >
          <Icon aria-hidden />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{tooltip ?? label}</TooltipContent>
    </Tooltip>
  );
});

function isEntry(
  item: RowActionItem
): item is Extract<RowActionItem, { label: string; onSelect: () => void }> {
  return item.kind === undefined || item.kind === 'item';
}

/** The items as DropdownMenu rows. */
export function RowActionMenuItems({ items }: { items: ReadonlyArray<RowActionItem> }) {
  return (
    <>
      {items.map((item, index) => {
        if (item.kind === 'separator') return <DropdownMenuSeparator key={`sep-${index}`} />;
        if (item.kind === 'label')
          return <DropdownMenuLabel key={`label-${index}`}>{item.label}</DropdownMenuLabel>;
        if (!isEntry(item)) return null;
        const Icon = item.icon;
        return (
          <DropdownMenuItem
            key={`${item.label}-${index}`}
            variant={item.destructive ? 'destructive' : 'default'}
            disabled={item.disabled}
            data-testid={item.testId}
            onSelect={() => item.onSelect()}
          >
            {Icon ? <Icon aria-hidden /> : null}
            <span className="min-w-0 flex-1 truncate">{item.label}</span>
            {item.shortcut ? <DropdownMenuShortcut>{item.shortcut}</DropdownMenuShortcut> : null}
          </DropdownMenuItem>
        );
      })}
    </>
  );
}

/** The same items as ContextMenu rows. */
export function RowContextMenuItems({ items }: { items: ReadonlyArray<RowActionItem> }) {
  return (
    <>
      {items.map((item, index) => {
        if (item.kind === 'separator') return <ContextMenuSeparator key={`sep-${index}`} />;
        if (item.kind === 'label')
          return <ContextMenuLabel key={`label-${index}`}>{item.label}</ContextMenuLabel>;
        if (!isEntry(item)) return null;
        const Icon = item.icon;
        return (
          <ContextMenuItem
            key={`${item.label}-${index}`}
            variant={item.destructive ? 'destructive' : 'default'}
            disabled={item.disabled}
            data-testid={item.testId}
            onSelect={() => item.onSelect()}
          >
            {Icon ? <Icon aria-hidden /> : null}
            <span className="min-w-0 flex-1 truncate">{item.label}</span>
            {item.shortcut ? (
              <span className="ml-auto text-supporting text-text-muted">{item.shortcut}</span>
            ) : null}
          </ContextMenuItem>
        );
      })}
    </>
  );
}

export interface RowActionsProps {
  /** One primary icon action (an `IconAction`), before the `⋯`. */
  primary?: React.ReactNode;
  /** The menu behind `⋯`. Pass the same array to `RowContextMenu`. */
  menu?: ReadonlyArray<RowActionItem>;
  /** Shown at rest; gives way to the actions on hover or focus (a relative time). */
  meta?: React.ReactNode;
  /** The `⋯` button's name and tooltip. */
  menuLabel?: string;
  /** Size of the `⋯` button; 32px (`default`) in 40px rows, `xs` in 28px rows. */
  size?: RowActionSize;
  onMenuOpenChange?: (open: boolean) => void;
  menuAlign?: 'start' | 'center' | 'end';
  className?: string;
  menuTestId?: string;
  'data-testid'?: string;
}

export function RowActions({
  primary,
  menu,
  meta,
  menuLabel = uiCopy.moreActions,
  size = 'default',
  onMenuOpenChange,
  menuAlign = 'end',
  className,
  menuTestId,
  'data-testid': testId,
}: RowActionsProps) {
  const [open, setOpen] = React.useState(false);
  const hasMenu = Boolean(menu && menu.length > 0);
  const handleOpenChange = (next: boolean) => {
    setOpen(next);
    onMenuOpenChange?.(next);
  };

  const cluster = (
    <div
      className="br-row-actions"
      data-slot="row-actions"
      data-state={open ? 'open' : 'closed'}
      data-testid={testId}
    >
      {primary}
      {hasMenu ? (
        <DropdownMenu open={open} onOpenChange={handleOpenChange}>
          <Tooltip>
            <TooltipTrigger asChild>
              <DropdownMenuTrigger asChild>
                <Button
                  type="button"
                  variant="ghost"
                  shape="round"
                  size={size}
                  aria-label={menuLabel}
                  data-testid={menuTestId}
                  data-row-action=""
                  onClick={(event) => event.stopPropagation()}
                >
                  <MoreHorizontal aria-hidden />
                </Button>
              </DropdownMenuTrigger>
            </TooltipTrigger>
            <TooltipContent>{menuLabel}</TooltipContent>
          </Tooltip>
          <DropdownMenuContent align={menuAlign}>
            <RowActionMenuItems items={menu!} />
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </div>
  );

  return (
    <div
      className={cn('br-row-actions-slot', className)}
      data-slot="row-actions-slot"
      data-has-meta={meta ? 'true' : undefined}
      data-state={open ? 'open' : 'closed'}
    >
      {meta ? (
        <span className="br-row-actions-meta text-supporting text-text-muted">{meta}</span>
      ) : null}
      {cluster}
    </div>
  );
}

export interface RowContextMenuProps {
  items: ReadonlyArray<RowActionItem>;
  /** The row: one element, which receives the context-menu handlers. */
  children: React.ReactElement;
  disabled?: boolean;
  onOpenChange?: (open: boolean) => void;
}

/**
 * Right-click, Shift+F10 and the ContextMenu key on the row open `items` as a ContextMenu. The
 * row is marked `data-row-actions-host`, which is what reveals its `RowActions` on hover.
 */
export function RowContextMenu({ items, children, disabled, onOpenChange }: RowContextMenuProps) {
  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (disabled || !isContextMenuKey(event)) return;
    // A key pressed inside an open menu (portalled, but still a React child) is the menu's.
    const target = event.target;
    if (target instanceof Element && target.closest('[data-radix-menu-content]')) return;
    event.preventDefault();
    openContextMenuFromKeyboard(
      event.currentTarget,
      target instanceof Element ? target : event.currentTarget
    );
  };
  return (
    <ContextMenu onOpenChange={onOpenChange}>
      <ContextMenuTrigger asChild disabled={disabled}>
        <Slot data-row-actions-host="" onKeyDown={onKeyDown}>
          {children}
        </Slot>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <RowContextMenuItems items={items} />
      </ContextMenuContent>
    </ContextMenu>
  );
}
