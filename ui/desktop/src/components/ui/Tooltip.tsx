import * as React from 'react';
import * as TooltipPrimitive from '@radix-ui/react-tooltip';

import { cn } from '../../utils';

/**
 * `--background-inverse` fill, `--text-inverse` 12/16, 6px×8px padding, no
 * arrow, 8px offset. `text-supporting` is the 12/16 role; `font-medium` is a
 * DELIBERATE override of its 400 weight — a tooltip sits on an inverse fill and
 * needs the extra weight to hold up, and that is what shipped before the roles
 * existed.
 *
 * Radius is `--radius-container` (12px), not the `--radius-inner` (4px) this
 * carried. Two steps off the ladder, against the repo's own written rules on
 * both sides: `main.css` reserves `--radius-inner` for "inline code, chips,
 * checkbox, swatches — nested inside a control", and the cohesion design says
 * "every floating thing — popover, dropdown, select, mention picker, toast,
 * tooltip — gets the same recipe … 12px radius". The old comment cited
 * "design.md §4.4" as authority for 4px; that document no longer exists.
 *
 * Z — deliberately `--z-modal-dropdown` (500), not `--z-dropdown` (200). A tooltip
 * must paint above whatever surface owns its trigger, and it is portalled to <body>
 * so it cannot know what that is. ContextWindowIndicator is the proof: its gauge
 * ("Compact chat", "Drag to adjust auto-compact threshold") renders INSIDE a
 * PopoverContent, which sits at 500 — at 200 those tooltips would hide behind the
 * very popover that contains them.
 */
export const TOOLTIP_SURFACE_CLASS_NAME =
  'bg-background-inverse text-text-inverse z-[var(--z-modal-dropdown)] w-max max-w-[min(20rem,calc(100vw-16px))] break-words rounded-container px-2 py-1.5 text-left font-sans text-supporting font-medium whitespace-normal';

function TooltipProvider({
  delayDuration = 500,
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Provider>) {
  return (
    <TooltipPrimitive.Provider
      data-slot="tooltip-provider"
      delayDuration={delayDuration}
      {...props}
    />
  );
}

/**
 * Whether the focus happening now was moved by the Tab key (Q2-56).
 *
 * Radix opens a trigger's tooltip on ANY focus that did not start with a pointer press, and the
 * app restores focus programmatically all the time — a dialog or menu closing hands it back to
 * its opener, a pane closing to its toggle. So "More actions", "Attach" and "Analysis Lab
 * options" popped their tooltips over the New label every time something closed (carol, dave,
 * frank, a11y). A focus the person moved with Tab still shows it, and hover is unchanged.
 *
 * One tracker for the whole document, installed once when this module loads, rather than a
 * listener per trigger. It is the channel header's rule (`useTooltipOnTabFocusOnly` in
 * `crew/channel/ChannelHeader.tsx`), moved here so every tooltip has it:
 *   - a Tab keydown marks the next focus as the person's — the focus move is that keydown's
 *     default action, so it lands while the mark is set, and a focus trap or menu that moves
 *     focus itself on Tab lands inside it too;
 *   - any keyup, pointer press or the window losing focus clears it, so a focus that arrives
 *     later (a restore after Escape or a click, the window coming back) is a program's.
 * Capture phase, so nothing that stops propagation can hide a Tab from it.
 */
let focusMovedByTab = false;

function installTabFocusTracker() {
  if (typeof document === 'undefined' || typeof window === 'undefined') return;
  const settle = () => {
    focusMovedByTab = false;
  };
  document.addEventListener(
    'keydown',
    (event) => {
      if (event.key === 'Tab') focusMovedByTab = true;
    },
    true
  );
  document.addEventListener('keyup', settle, true);
  document.addEventListener('pointerdown', settle, true);
  window.addEventListener('blur', settle);
}

installTabFocusTracker();

/** For tests and callers that need the same verdict: was the focus in progress moved by Tab? */
export function isFocusFromTabKey(): boolean {
  return focusMovedByTab;
}

/**
 * Set by a trigger's focus handler, just before Radix's own asks to open, when that focus was not
 * a Tab. `Tooltip` reads it in `onOpenChange` and declines that one open. It is cleared on the
 * next microtask, so it can only ever refuse the open the same focus event asked for.
 */
const TooltipFocusGateContext = React.createContext<React.RefObject<boolean> | null>(null);

/**
 * The tooltip root, holding its own open state (controlled or not, as before) so it can decline
 * an open that a programmatic focus asked for. The trigger's focus event is deliberately NOT
 * cancelled to achieve this: a cancelled focus event skips every handler Radix composes after
 * the caller's, and a tooltip wraps triggers that act on focus themselves — a tab activates, a
 * roving toolbar item records its stop.
 */
function Tooltip({
  open: openProp,
  defaultOpen = false,
  onOpenChange,
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Root>) {
  const [uncontrolledOpen, setUncontrolledOpen] = React.useState(defaultOpen);
  const controlled = openProp !== undefined;
  const open = controlled ? openProp : uncontrolledOpen;
  const programmaticFocus = React.useRef(false);

  const handleOpenChange = React.useCallback(
    (next: boolean) => {
      if (next && programmaticFocus.current) {
        programmaticFocus.current = false;
        return;
      }
      if (!controlled) setUncontrolledOpen(next);
      onOpenChange?.(next);
    },
    [controlled, onOpenChange]
  );

  return (
    <TooltipProvider>
      <TooltipFocusGateContext.Provider value={programmaticFocus}>
        <TooltipPrimitive.Root
          data-slot="tooltip"
          open={open}
          onOpenChange={handleOpenChange}
          {...props}
        />
      </TooltipFocusGateContext.Provider>
    </TooltipProvider>
  );
}

function TooltipTrigger({
  onFocus,
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Trigger>) {
  const programmaticFocus = React.useContext(TooltipFocusGateContext);
  // Runs before Radix's own focus handler (Radix composes the caller's first), so the gate is
  // set by the time Radix asks the root to open.
  const handleFocus = (event: React.FocusEvent<HTMLButtonElement>) => {
    onFocus?.(event);
    if (!programmaticFocus || isFocusFromTabKey()) return;
    programmaticFocus.current = true;
    queueMicrotask(() => {
      programmaticFocus.current = false;
    });
  };
  return <TooltipPrimitive.Trigger data-slot="tooltip-trigger" onFocus={handleFocus} {...props} />;
}

function TooltipContent({
  className,
  sideOffset = 8,
  children,
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Content>) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        data-slot="tooltip-content"
        sideOffset={sideOffset}
        className={cn(
          TOOLTIP_SURFACE_CLASS_NAME,
          'animate-in fade-in-0 duration-[120ms] data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:duration-[120ms]',
          className
        )}
        {...props}
      >
        {children}
      </TooltipPrimitive.Content>
    </TooltipPrimitive.Portal>
  );
}

export { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider };
