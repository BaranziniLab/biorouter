'use client';

import * as React from 'react';
import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu';
import { CheckIcon, ChevronRightIcon, CircleIcon } from '../icons/app-icons';

import { cn } from '../../utils';

/**
 * What `DropdownMenuContent` needs from the menu around it to let Tab leave (Q2-50): a way to
 * close the whole menu, and the trigger the Tab continues from.
 */
type DropdownMenuTabOut = {
  close: () => void;
  triggerRef: React.RefObject<HTMLElement | null>;
  /** True while a Tab-out is closing the menu, so the focus-outside dismissal it provokes is not
   * reported to the caller as a second close. */
  closingRef: React.RefObject<boolean>;
};

const DropdownMenuTabOutContext = React.createContext<DropdownMenuTabOut | null>(null);

/**
 * The menu root. It holds the open state itself (controlled or not, as before) only so the
 * content can close it on Tab: Radix exposes no close from inside a menu, and its own Tab
 * handling just swallows the key.
 */
function DropdownMenu({
  modal = false,
  open: openProp,
  defaultOpen = false,
  onOpenChange,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.Root>) {
  const [uncontrolledOpen, setUncontrolledOpen] = React.useState(defaultOpen);
  const controlled = openProp !== undefined;
  const open = controlled ? openProp : uncontrolledOpen;
  const triggerRef = React.useRef<HTMLElement | null>(null);
  const closingRef = React.useRef(false);

  const handleOpenChange = React.useCallback(
    (next: boolean) => {
      if (!next && closingRef.current) return;
      if (!controlled) setUncontrolledOpen(next);
      onOpenChange?.(next);
    },
    [controlled, onOpenChange]
  );

  const tabOut = React.useMemo<DropdownMenuTabOut>(
    () => ({ close: () => handleOpenChange(false), triggerRef, closingRef }),
    [handleOpenChange]
  );

  return (
    <DropdownMenuTabOutContext.Provider value={tabOut}>
      <DropdownMenuPrimitive.Root
        data-slot="dropdown-menu"
        modal={modal}
        open={open}
        onOpenChange={handleOpenChange}
        {...props}
      />
    </DropdownMenuTabOutContext.Provider>
  );
}

function DropdownMenuPortal({
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.Portal>) {
  return <DropdownMenuPrimitive.Portal data-slot="dropdown-menu-portal" {...props} />;
}

function DropdownMenuTrigger({
  ref,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.Trigger>) {
  const tabOut = React.useContext(DropdownMenuTabOutContext);
  // The trigger is where a Tab out of the open menu continues from, so the menu records it.
  const setTrigger = React.useCallback(
    (node: HTMLButtonElement | null) => {
      if (tabOut) tabOut.triggerRef.current = node;
      if (typeof ref === 'function') ref(node);
      else if (ref) ref.current = node;
    },
    [ref, tabOut]
  );
  return (
    <DropdownMenuPrimitive.Trigger data-slot="dropdown-menu-trigger" ref={setTrigger} {...props} />
  );
}

/** What can take focus in sequential (Tab) order. */
const TABBABLE_CANDIDATES =
  'a[href], area[href], button, input, select, textarea, iframe, summary, [tabindex], [contenteditable]:not([contenteditable="false"])';

function isTabbable(element: HTMLElement): boolean {
  if (element.tabIndex < 0) return false;
  if (element.matches(':disabled')) return false;
  if (element instanceof HTMLInputElement && element.type === 'hidden') return false;
  if (element.closest('[inert], [hidden]')) return false;
  // Radix's focus guards bracket <body> while a layer is open; they are not places to land.
  if (element.hasAttribute('data-radix-focus-guard')) return false;
  const visible = (
    element as HTMLElement & { checkVisibility?: (options?: object) => boolean }
  ).checkVisibility?.({ visibilityProperty: true });
  if (visible === false) return false;
  // One stop per radio group: its checked radio, or every radio while none is checked.
  if (element instanceof HTMLInputElement && element.type === 'radio' && element.name) {
    if (!element.checked) {
      const group = Array.from(
        element.ownerDocument.querySelectorAll<HTMLInputElement>('input[type="radio"]')
      ).filter((radio) => radio.name === element.name && radio.form === element.form);
      if (group.some((radio) => radio.checked)) return false;
    }
  }
  return true;
}

/**
 * The dialog a trigger sits in — a modal or non-modal Radix `Dialog`, an `AlertDialog`, a `Sheet`,
 * or a `Popover` (Radix renders its content as `role="dialog"` too).
 */
const FOCUS_CONTAINER = '[role="dialog"], [role="alertdialog"]';

/**
 * Where Tab (or Shift+Tab) goes from `from`, skipping anything `skip` rejects: the next stop in
 * sequential focus order, positive `tabindex` first as the browser orders it. `from` need not be
 * a stop itself (a roving trigger carries `tabindex="-1"`), in which case the answer is the first
 * stop after it, or the last before it.
 *
 * Inside a dialog the search never leaves the dialog, and it wraps at the dialog's edges: that is
 * what Tab does there with no menu open, because every Radix dialog and popover mounts its
 * `FocusScope` with `loop`. The document-wide search this replaced sent a Shift+Tab from a
 * dialog's FIRST control to the page behind it (PermissionModal: the first tool's permission
 * menu, then Shift+Tab, landed on Settings). The dialog's trap could not stop that — an open menu
 * pauses it — and does not pull focus back when it resumes. Two things it must not do instead:
 * - skip `aria-hidden` elements. A modal menu hides the WHOLE page, the dialog included, so that
 *   would skip every stop there is; the page behind a dialog is shut out by the scope, not by
 *   `aria-hidden`.
 * - return null at an edge. The caller falls back to the trigger, which is correct but strands
 *   the Tab; a wrap is where Tab really goes.
 * A hand-rolled `role="dialog"` panel that does not trap is scoped the same way: keeping focus in
 * the panel is the safe failure, and none of those panels holds a menu today.
 */
function sequentialNeighbour(
  from: HTMLElement,
  backward: boolean,
  skip: (element: HTMLElement) => boolean
): HTMLElement | null {
  const scope = from.closest<HTMLElement>(FOCUS_CONTAINER);
  const root: Document | HTMLElement = scope ?? from.ownerDocument;
  const candidates = Array.from(root.querySelectorAll<HTMLElement>(TABBABLE_CANDIDATES)).filter(
    (element) => element === from || (!skip(element) && isTabbable(element))
  );
  const order = [
    ...candidates.filter((element) => element.tabIndex > 0).sort((a, b) => a.tabIndex - b.tabIndex),
    ...candidates.filter((element) => element.tabIndex <= 0),
  ];
  const at = order.indexOf(from);
  let stops: HTMLElement[];
  let next: HTMLElement | undefined;
  if (at !== -1 && from.tabIndex >= 0) {
    stops = order;
    next = backward ? order[at - 1] : order[at + 1];
  } else {
    stops = order.filter((element) => element !== from);
    const after = stops.findIndex(
      (element) => from.compareDocumentPosition(element) & Node.DOCUMENT_POSITION_FOLLOWING
    );
    if (backward) next = after === -1 ? stops[stops.length - 1] : stops[after - 1];
    else next = after === -1 ? undefined : stops[after];
  }
  if (next) return next;
  // Off the edge of a dialog: round to its other end, as its focus scope's `loop` does.
  if (scope) return (backward ? stops[stops.length - 1] : stops[0]) ?? null;
  return null;
}

/**
 * Tab and Shift+Tab leave an open menu (Q2-50; WAI-ARIA APG menu button): the menu closes and
 * focus goes where Tab would have gone from the TRIGGER — the next stop after it, or the one
 * before it, never out of a dialog the trigger sits in (see `sequentialNeighbour`) — so the
 * keyboard user is never parked on a menu they meant to pass.
 *
 * Radix swallows Tab inside a menu (`MenuContentImpl`: `if (event.key === "Tab")
 * event.preventDefault()`), so only Escape used to leave; three critics called that a trap. The
 * browser cannot be asked to "carry on" a Tab that Radix has already cancelled, so the move is
 * made here, from the trigger. A Tab bubbling up from a submenu closes the whole menu, as the
 * APG asks; a Tab the caller's own `onKeyDown` cancelled is left alone.
 *
 * Returns the `onCloseAutoFocus` half: when the content finally unmounts (after its exit
 * animation) Radix would put focus back on the trigger, which would undo the Tab. That is
 * cancelled — and if focus was lost meanwhile (a modal menu's focus trap pulls a focus that
 * leaves it back inside, and the item then unmounts), the destination is focused then.
 */
function useTabLeavesMenu(
  onKeyDown: React.KeyboardEventHandler<HTMLDivElement> | undefined,
  onCloseAutoFocus: ((event: Event) => void) | undefined
) {
  const tabOut = React.useContext(DropdownMenuTabOutContext);
  const destinationRef = React.useRef<HTMLElement | null>(null);

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const cancelledBefore = event.defaultPrevented;
    onKeyDown?.(event);
    if (!tabOut || event.key !== 'Tab') return;
    if (event.ctrlKey || event.metaKey || event.altKey || event.nativeEvent.isComposing) return;
    if (!cancelledBefore && event.defaultPrevented) return;
    event.preventDefault();

    const content = event.currentTarget;
    const labelledBy = content.getAttribute('aria-labelledby');
    const trigger =
      tabOut.triggerRef.current ??
      (labelledBy ? content.ownerDocument.getElementById(labelledBy) : null);
    const destination = trigger
      ? (sequentialNeighbour(trigger, event.shiftKey, (element) =>
          Boolean(element.closest('[data-radix-menu-content]'))
        ) ?? trigger)
      : null;

    destinationRef.current = destination;
    tabOut.closingRef.current = false;
    tabOut.close();
    // Focusing outside a non-modal menu makes Radix dismiss it again; that is this same close.
    tabOut.closingRef.current = true;
    try {
      destination?.focus();
    } finally {
      tabOut.closingRef.current = false;
    }
  };

  const handleCloseAutoFocus = (event: Event) => {
    onCloseAutoFocus?.(event);
    const destination = destinationRef.current;
    if (!destination) return;
    destinationRef.current = null;
    event.preventDefault();
    const active = destination.ownerDocument.activeElement;
    if (destination.isConnected && (!active || active === destination.ownerDocument.body)) {
      destination.focus();
    }
  };

  return { handleKeyDown, handleCloseAutoFocus };
}

/**
 * The menu's enter and exit use the app's `--ease-out` (Q2-50). `tw-animate-css`'s
 * `animate-in`/`animate-out` read `--tw-ease` and fall back to the browser's `ease`, which is
 * what every menu shipped with; `ease-[var(--ease-out)]` sets `--tw-ease`. It is the class the
 * sidebar already carries, so it is known to be generated.
 */
const MENU_EASE_CLASS_NAME = 'ease-[var(--ease-out)]';

function DropdownMenuContent({
  className,
  sideOffset = 6,
  onKeyDown,
  onCloseAutoFocus,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.Content>) {
  const { handleKeyDown, handleCloseAutoFocus } = useTabLeavesMenu(onKeyDown, onCloseAutoFocus);
  return (
    <DropdownMenuPrimitive.Portal>
      <DropdownMenuPrimitive.Content
        data-slot="dropdown-menu-content"
        sideOffset={sideOffset}
        onKeyDown={handleKeyDown}
        onCloseAutoFocus={handleCloseAutoFocus}
        // design.md §4.5: --radius-container (12px) surface, 4px padding, 6px trigger offset.
        //
        // Z — deliberately --z-modal-dropdown (500), not --z-dropdown (200): this
        // content is portalled to <body>, making it a SIBLING of any dialog content
        // (--z-modal, 400) it is rendered inside, with no way to detect that host.
        // PermissionModal.tsx renders a DropdownMenuContent inside a DialogContent;
        // at 200 that menu would paint under the dialog that owns it.
        className={cn(
          'biorouter-popover-surface bg-background-default text-text-default data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[state=open]:duration-[var(--motion-base)] data-[state=closed]:duration-[var(--motion-fast)] data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 z-[var(--z-modal-dropdown)] max-h-(--radix-dropdown-menu-content-available-height) min-w-[8rem] origin-(--radix-dropdown-menu-content-transform-origin) overflow-x-hidden overflow-y-auto rounded-container p-1 space-y-0.5',
          MENU_EASE_CLASS_NAME,
          className
        )}
        {...props}
      />
    </DropdownMenuPrimitive.Portal>
  );
}

function DropdownMenuGroup({ ...props }: React.ComponentProps<typeof DropdownMenuPrimitive.Group>) {
  return <DropdownMenuPrimitive.Group data-slot="dropdown-menu-group" {...props} />;
}

/**
 * design.md §4.5 — THE menu row: 32px tall, 12px horizontal padding, `--radius-element`,
 * `text-secondary` (13/18), highlight fill `--overlay-hover`.
 *
 * `min-h-control-md` is what actually lands the 32px spec: 13/18 text inside 6px of vertical
 * padding measures 30px, and `items-center` then optically centres the label in the
 * extra 2px. Expressing height as a minimum (rather than a fixed `h-control-md`) lets a row
 * that wraps — a long extension name, a two-line label — grow instead of clipping.
 *
 * Every row-shaped member of this menu — item, checkbox, radio, sub-trigger — composes
 * this ONE string, so the per-call-site drift §4.5 called out cannot reappear. Call
 * sites should add only what is theirs (a toggle's `justify-between`, a cursor); a
 * call site that re-states padding or type size is reintroducing the drift.
 */
export const DROPDOWN_ROW_CLASS_NAME =
  "relative flex min-h-control-md cursor-default items-center gap-2 rounded-element px-3 py-1.5 text-secondary select-none transition-colors focus:bg-overlay-hover focus:text-text-default data-[disabled]:pointer-events-none data-[disabled]:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4";

function DropdownMenuItem({
  className,
  inset,
  variant = 'default',
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.Item> & {
  inset?: boolean;
  variant?: 'default' | 'destructive';
}) {
  return (
    <DropdownMenuPrimitive.Item
      data-slot="dropdown-menu-item"
      data-inset={inset}
      data-variant={variant}
      className={cn(
        DROPDOWN_ROW_CLASS_NAME,
        "data-[variant=destructive]:text-text-danger data-[variant=destructive]:focus:bg-background-danger/10 dark:data-[variant=destructive]:focus:bg-background-danger/20 data-[variant=destructive]:focus:text-text-danger data-[variant=destructive]:*:[svg]:!text-text-danger [&_svg:not([class*='text-'])]:text-text-muted data-[inset]:pl-8",
        className
      )}
      {...props}
    />
  );
}

function DropdownMenuCheckboxItem({
  className,
  children,
  checked,
  showIndicator = true,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.CheckboxItem> & {
  showIndicator?: boolean;
}) {
  return (
    <DropdownMenuPrimitive.CheckboxItem
      data-slot="dropdown-menu-checkbox-item"
      className={cn(
        DROPDOWN_ROW_CLASS_NAME,
        // The left gutter exists only to hold the check; without an indicator the
        // row keeps the plain §4.5 padding instead of a phantom 32px indent.
        showIndicator && 'pl-8',
        className
      )}
      checked={checked}
      {...props}
    >
      {showIndicator && (
        <span className="pointer-events-none absolute left-2 flex size-3.5 items-center justify-center">
          <DropdownMenuPrimitive.ItemIndicator>
            <CheckIcon className="size-4" />
          </DropdownMenuPrimitive.ItemIndicator>
        </span>
      )}
      {children}
    </DropdownMenuPrimitive.CheckboxItem>
  );
}

function DropdownMenuRadioGroup({
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.RadioGroup>) {
  return <DropdownMenuPrimitive.RadioGroup data-slot="dropdown-menu-radio-group" {...props} />;
}

function DropdownMenuRadioItem({
  className,
  children,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.RadioItem>) {
  return (
    <DropdownMenuPrimitive.RadioItem
      data-slot="dropdown-menu-radio-item"
      className={cn(DROPDOWN_ROW_CLASS_NAME, 'pl-8', className)}
      {...props}
    >
      <span className="pointer-events-none absolute left-2 flex size-3.5 items-center justify-center">
        <DropdownMenuPrimitive.ItemIndicator>
          <CircleIcon className="size-2 fill-current" />
        </DropdownMenuPrimitive.ItemIndicator>
      </span>
      {children}
    </DropdownMenuPrimitive.RadioItem>
  );
}

function DropdownMenuLabel({
  className,
  inset,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.Label> & {
  inset?: boolean;
}) {
  return (
    <DropdownMenuPrimitive.Label
      data-slot="dropdown-menu-label"
      data-inset={inset}
      // design.md §4.5 section label: `text-caps` (11px, 500, +0.08em), --text-muted, 8px×12px.
      // It names a group; it is not a row, so it never takes the row height or hover.
      // No `uppercase` beside `text-caps` — the role carries the transform itself.
      className={cn('px-3 py-1.5 text-caps text-text-muted data-[inset]:pl-8', className)}
      {...props}
    />
  );
}

function DropdownMenuSeparator({
  className,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.Separator>) {
  return (
    <DropdownMenuPrimitive.Separator
      data-slot="dropdown-menu-separator"
      // --border-subtle, 4px margin (§4.5). `bg-border-default` is slated for deletion.
      className={cn('bg-border-subtle -mx-1 my-1 h-px', className)}
      {...props}
    />
  );
}

function DropdownMenuShortcut({ className, ...props }: React.ComponentProps<'span'>) {
  return (
    <span
      data-slot="dropdown-menu-shortcut"
      className={cn('text-text-muted ml-auto text-supporting tracking-widest', className)}
      {...props}
    />
  );
}

function DropdownMenuSub({ ...props }: React.ComponentProps<typeof DropdownMenuPrimitive.Sub>) {
  return <DropdownMenuPrimitive.Sub data-slot="dropdown-menu-sub" {...props} />;
}

function DropdownMenuSubTrigger({
  className,
  inset,
  children,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.SubTrigger> & {
  inset?: boolean;
}) {
  return (
    <DropdownMenuPrimitive.SubTrigger
      data-slot="dropdown-menu-sub-trigger"
      data-inset={inset}
      className={cn(
        DROPDOWN_ROW_CLASS_NAME,
        'data-[state=open]:bg-overlay-hover data-[state=open]:text-text-default data-[inset]:pl-8',
        className
      )}
      {...props}
    >
      {children}
      <ChevronRightIcon className="ml-auto size-4" />
    </DropdownMenuPrimitive.SubTrigger>
  );
}

function DropdownMenuSubContent({
  className,
  ...props
}: React.ComponentProps<typeof DropdownMenuPrimitive.SubContent>) {
  return (
    <DropdownMenuPrimitive.SubContent
      data-slot="dropdown-menu-sub-content"
      className={cn(
        'biorouter-popover-surface bg-background-default text-text-default data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[state=open]:duration-[var(--motion-base)] data-[state=closed]:duration-[var(--motion-fast)] data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 z-[var(--z-modal-dropdown)] min-w-[8rem] origin-(--radix-dropdown-menu-content-transform-origin) overflow-hidden rounded-container p-1 space-y-0.5',
        MENU_EASE_CLASS_NAME,
        className
      )}
      {...props}
    />
  );
}

export {
  DropdownMenu,
  DropdownMenuPortal,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuItem,
  DropdownMenuCheckboxItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuSub,
  DropdownMenuSubTrigger,
  DropdownMenuSubContent,
};
