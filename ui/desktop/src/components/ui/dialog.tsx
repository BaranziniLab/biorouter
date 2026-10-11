'use client';

import * as React from 'react';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import { XIcon } from '../icons/app-icons';

import { cn } from '../../utils';
import { installDialogTabRepair } from './dialogTabRepair';
import { focusArrivedFrom, lastFocusOutsideMenu, lastInputWasKey } from './inputTrail';

/**
 * The control that stands for `element` as the opener of a dialog: the element itself, or, when
 * it sits in a menu, the control that opened that menu. A menu item is gone by the time the
 * dialog it opened closes (the menu closes as it acts), so the item is not an opener; the menu
 * names its own, because Radix labels a DropdownMenu's content by its trigger
 * (`aria-labelledby`), and a submenu is labelled by its sub-trigger, which is itself in a menu,
 * hence the climb. A ContextMenu names nothing, so its opener is the control that held focus when
 * the menu took it (`lastFocusOutsideMenu`: the row a Shift+F10 was pressed on).
 *
 * `viaMenu` says a menu was crossed, which is what changes the rules (see the hook).
 */
export function resolveDialogOpener(element: Element | null): {
  opener: HTMLElement | null;
  viaMenu: boolean;
} {
  let current: Element | null = element;
  let viaMenu = false;
  // A submenu nests a few levels; the bound only stops a malformed tree from looping.
  for (let depth = 0; depth < 8; depth += 1) {
    if (!(current instanceof HTMLElement) || current === document.body) {
      return { opener: null, viaMenu };
    }
    const menu = current.closest('[role="menu"]');
    if (!menu) return { opener: current, viaMenu };
    viaMenu = true;
    const labelId = menu.getAttribute('aria-labelledby')?.split(/\s+/)[0];
    let label = labelId ? document.getElementById(labelId) : null;
    // A caller that puts its own `id` on the trigger replaces the one Radix names the menu after
    // (the menu's label then dangles), but the trigger still `aria-controls` the open menu.
    if (!label && menu.id) {
      const controller = document.querySelector(
        `[aria-controls="${menu.id.replace(/["\\]/g, '\\$&')}"]`
      );
      if (controller && !menu.contains(controller)) label = controller as HTMLElement;
    }
    if (!label || menu.contains(label)) {
      // A menu that names no opener (a ContextMenu): the control that held focus when it opened.
      const heldFocus = lastFocusOutsideMenu();
      return { opener: heldFocus && !menu.contains(heldFocus) ? heldFocus : null, viaMenu };
    }
    current = label;
  }
  return { opener: null, viaMenu };
}

/**
 * A field the browser draws its own focus for: a text box takes the fill-and-edge shift in every
 * input modality, so a dialog whose first stop is one has nothing to hide after a pointer press.
 */
const TEXT_ENTRY =
  'input:not([type="hidden"]):not([type="checkbox"]):not([type="radio"]):not([type="range"]):not([type="button"]):not([type="submit"]):not([type="reset"]):not([type="file"]):not([type="image"]):not([disabled]), textarea:not([disabled]), [contenteditable]:not([contenteditable="false"])';

/**
 * Whether `element` is a text field in that sense. The shared `Select` types to filter through an
 * `<input>`, but it reads as a closed control: focused by a script after a mouse open it sat in the
 * grey focus surface like a choice the person had just made (V3-20), so it is not a field here.
 */
function isTextField(element: Element): boolean {
  return element.matches(TEXT_ENTRY) && !element.closest('.br-select');
}

/**
 * Radix's `getComputedStyle` test for a candidate it cannot focus (its focus scope's `isHidden`):
 * `visibility: hidden` on the node, or `display: none` on it or any ancestor inside the content.
 */
function isHiddenIn(node: HTMLElement, container: HTMLElement): boolean {
  if (getComputedStyle(node).visibility === 'hidden') return true;
  for (let current: HTMLElement | null = node; current; current = current.parentElement) {
    if (current === container) return false;
    if (getComputedStyle(current).display === 'none') return true;
  }
  return false;
}

/**
 * The control Radix's focus scope focuses when a dialog opens and nobody places focus: the first
 * tabbable node in document order, links left out, that can take focus. The same walk as Radix's
 * `getTabbableCandidates` + `removeLinks` + `focusFirst`, read before Radix acts so the dialog can
 * decide not to let it.
 */
function firstAutoFocusTarget(container: HTMLElement): HTMLElement | null {
  const walker = document.createTreeWalker(container, NodeFilter.SHOW_ELEMENT, {
    acceptNode: (node) => {
      const element = node as HTMLElement & { disabled?: boolean; type?: string };
      const hiddenInput = element.tagName === 'INPUT' && element.type === 'hidden';
      if (element.disabled || element.hidden || hiddenInput) return NodeFilter.FILTER_SKIP;
      return element.tabIndex >= 0 ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP;
    },
  });
  while (walker.nextNode()) {
    const candidate = walker.currentNode as HTMLElement;
    if (candidate.tagName === 'A' || isHiddenIn(candidate, container)) continue;
    return candidate;
  }
  return null;
}

/**
 * Give the keyboard focus back to the control that opened a dialog or a sheet (WCAG 2.4.3; the
 * ARIA dialog pattern). Radix's modal content, when it closes, focuses its own `Trigger` and
 * nothing else: it cancels the focus scope's usual return to whatever was focused before. A
 * dialog opened from state, with no Trigger (every one a button's `onClick` mounts), therefore
 * left focus on `<body>` after Escape, and a keyboard user began again at the top of the page.
 * Settings once fixed it dialog by dialog (T3-SH-12); this is the same rule for every dialog and
 * sheet at once.
 *
 * The opener is read in `onOpenAutoFocus`: Radix fires it when the content mounts, before it moves
 * focus inside, so `document.activeElement` is still the control that was pressed. (It cannot be
 * read when `DialogContent` first renders: this wrapper is mounted while the dialog is still
 * closed, and only the Radix content inside it mounts on open.) The caller's own handlers run
 * first, and `onCloseAutoFocus` wins when it cancels the event (Crew's dialogs return focus
 * themselves and say so). An opener that has gone by the time the dialog closes (a row that
 * re-rendered) is left alone, and Radix does what it would have done. No ring is forced: whether
 * returned focus reads as keyboard focus is the browser's call from how the person closed it (a
 * click on Cancel shows none, Escape does).
 *
 * **A dialog opened from a menu item** (sidebar row actions, "View info", "Delete"). The item is
 * what holds focus when the dialog mounts, so the opener is the control that opened the menu
 * (`resolveDialogOpener`), and the menus' own rule applies: a pick made with the KEYBOARD gives
 * focus back when the dialog is closed with the keyboard (⋯, Enter, Enter on "View info", Escape
 * lands on ⋯, not on `<body>`: V4-03); a pick made with the POINTER does not, because a
 * script-focused ⋯ would wear a keyboard ring after a mouse click (`chats/menuFocusReturn.ts`).
 *
 * **A pointer open is a quiet open**, from a menu or not. Whether a focus a script moves matches
 * `:focus-visible` is Chromium's call from the focus it replaces, not from the input that caused
 * it: after a pointer pick in a menu every script focus matches (the menu item decides, V4-04),
 * and a click on a trigger that ALREADY holds keyboard focus (Escape gave it back with a ring, then
 * the mouse opened the dialog again) changes no focus, so the browser keeps the keyboard modality
 * and Radix's auto-focus inherits the ring (V4-R5-1: Reset data's "Select all", Configuration's
 * "Go to Privacy", Usage's InfoTip). So after any pointer open, unless the first stop is a text
 * field, focus goes to the dialog box, which draws no ring (`.biorouter-modal-surface:focus` in
 * `main.css`) and is where a screen reader reads the title and the message; the next Tab reaches
 * the first control, ringed. A text field keeps focus (a field is always ringed and wants the
 * cursor). A KEY open keeps Radix's first stop, because there the ring is the point. A caller that
 * cancels the event and places focus itself is left alone.
 */
export function useReturnFocusToOpener(handlers: {
  onOpenAutoFocus?: (event: Event) => void;
  onCloseAutoFocus?: (event: Event) => void;
}): {
  onOpenAutoFocus: (event: Event) => void;
  onCloseAutoFocus: (event: Event) => void;
  recordFocusAlreadyInside: (content: HTMLElement | null) => void;
} {
  const { onOpenAutoFocus, onCloseAutoFocus } = handlers;
  const opener = React.useRef<HTMLElement | null>(null);
  // Set when the opener was found through a menu; true only if the pick was a key press too.
  const fromMenu = React.useRef<{ pickedByKey: boolean } | null>(null);
  // The content node an opener was last read for, so a ref that re-attaches mid-dialog (a caller's
  // inline ref) does not read a later focus move (back from a Select's portal) as the opener.
  const readFor = React.useRef<HTMLElement | null>(null);
  /**
   * A field with React's `autoFocus` takes focus as the content commits, before Radix's focus
   * scope mounts; the scope then finds focus inside and never fires `onOpenAutoFocus`, so the
   * opener was never read and Escape left focus on `<body>` (WS-SKILLS-2, measured on the Skills
   * page). The content's ref attaches after that focus and before the scope's effect: when focus
   * is already inside, the opener is the control the focus left to get there.
   */
  const recordFocusAlreadyInside = React.useCallback((content: HTMLElement | null) => {
    if (!content || readFor.current === content) return;
    const active = document.activeElement;
    if (!active || active === document.body || !content.contains(active)) return;
    const from = focusArrivedFrom(active);
    if (!from || content.contains(from)) return;
    readFor.current = content;
    const resolved = resolveDialogOpener(from);
    opener.current = resolved.opener;
    fromMenu.current = resolved.viaMenu ? { pickedByKey: lastInputWasKey() } : null;
  }, []);
  const handleOpen = React.useCallback(
    (event: Event) => {
      const content =
        event.currentTarget instanceof HTMLElement
          ? event.currentTarget
          : event.target instanceof HTMLElement
            ? event.target
            : null;
      readFor.current = content;
      const resolved = resolveDialogOpener(document.activeElement);
      opener.current = resolved.opener;
      const pickedByKey = lastInputWasKey();
      fromMenu.current = resolved.viaMenu ? { pickedByKey } : null;
      onOpenAutoFocus?.(event);
      if (event.defaultPrevented || pickedByKey || !content) return;
      const first = firstAutoFocusTarget(content);
      if (first && isTextField(first)) return;
      event.preventDefault();
      content.focus({ preventScroll: true });
    },
    [onOpenAutoFocus]
  );
  const handleClose = React.useCallback(
    (event: Event) => {
      onCloseAutoFocus?.(event);
      const target = opener.current;
      const menu = fromMenu.current;
      opener.current = null;
      fromMenu.current = null;
      if (event.defaultPrevented || !target || !target.isConnected) return;
      // Picked or closed with the pointer: nothing to give back (see above).
      if (menu && !(menu.pickedByKey && lastInputWasKey())) return;
      // Focus already sitting on something outside the content (a non-modal dialog closed by a
      // press on another field) is where the person went: leave it. Inside the content, or on
      // `<body>` once the focused control unmounted, is the case this exists for.
      const active = document.activeElement;
      const content = event.target instanceof Node ? event.target : null;
      if (active && active !== document.body && content && !content.contains(active)) return;
      event.preventDefault();
      target.focus();
    },
    [onCloseAutoFocus]
  );
  return { onOpenAutoFocus: handleOpen, onCloseAutoFocus: handleClose, recordFocusAlreadyInside };
}

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
      'biorouter-modal-overlay data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 fixed inset-0 z-[var(--z-modal)]',
      className
    )}
    {...props}
  />
));
DialogOverlay.displayName = DialogPrimitive.Overlay.displayName;

/**
 * Whether a pointer target sits in react-toastify's layer. `.Toastify` is the
 * container the library renders (`App.tsx`'s `ToastContainer`); the app's own
 * card class replaces the library's per-toast class, so the container is the
 * one stable hook (see `toastLayer.test.ts`).
 */
function isInsideToastLayer(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest('.Toastify') !== null;
}

function DialogContent({
  className,
  children,
  dismissible = true,
  showCloseButton = dismissible,
  onEscapeKeyDown,
  onPointerDownOutside,
  onOpenAutoFocus,
  onCloseAutoFocus,
  ref,
  ...props
}: React.ComponentProps<typeof DialogPrimitive.Content> & {
  dismissible?: boolean;
  showCloseButton?: boolean;
}) {
  const returnFocus = useReturnFocusToOpener({ onOpenAutoFocus, onCloseAutoFocus });
  const { recordFocusAlreadyInside } = returnFocus;
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
      recordFocusAlreadyInside(node);
      const teardown = installDialogTabRepair(node);
      // React 19 calls a ref callback's cleanup INSTEAD of re-invoking it with
      // `null`, so the forwarded ref has to be cleared here or a caller holding
      // one would keep a detached node.
      return () => {
        teardown();
        forward(null);
      };
    },
    [ref, recordFocusAlreadyInside]
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
        onOpenAutoFocus={returnFocus.onOpenAutoFocus}
        onCloseAutoFocus={returnFocus.onCloseAutoFocus}
        data-slot="dialog-content"
        className={cn(
          // No radius utility here on purpose: `.biorouter-modal-surface` (main.css,
          // unlayered, so it outranks Tailwind's utilities layer) already owns the
          // 16px dialog corner. A `rounded-*` here could only duplicate it or lie.
          // Motion (spec 2.6) is `.br-dialog-motion` in main.css: opacity and scale(.98) to 1
          // over `--dur-med-min` in, back to .98 over `--dur-fast` out, `--ease-out`; the
          // tw-animate utilities are only its carriers.
          'biorouter-modal-surface br-dialog-motion bg-background-default data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 fixed top-[50%] left-[50%] z-[var(--z-modal)] grid w-full max-w-[calc(100%-2rem)] translate-x-[-50%] translate-y-[-50%] gap-4 p-6 sm:max-w-lg',
          className
        )}
        onEscapeKeyDown={(event) => {
          onEscapeKeyDown?.(event);
          if (!dismissible) event.preventDefault();
        }}
        onPointerDownOutside={(event) => {
          onPointerDownOutside?.(event);
          if (!dismissible) event.preventDefault();
          // A press on a TOAST is not a press on the backdrop. Toasts sit above
          // every modal (`--z-toast` > `--z-modal`) and are rendered outside
          // any dialog's tree, so to Radix a click on one is "outside" — and
          // it closed the dialog underneath while the toast, whose × had never
          // received the click, stayed on screen. Measured on 2026-09-13
          // (defect D3b): a busy-store error over "Make this chat public?",
          // one click on the toast's ×, the dialog gone and the error still
          // there a minute later. `main.css` makes the toast layer take the
          // click at all; this stops the dialog treating it as a dismissal.
          if (isInsideToastLayer(event.target)) event.preventDefault();
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
