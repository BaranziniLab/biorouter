import * as React from 'react';
import { isFocusFromTabKey } from './Tooltip';

/**
 * The label hint on a trigger that opens a menu or a popover: "hover peeks, click pins" for an
 * ACTION trigger (H2). The hint names the control on a hover, closes on the press, never pins,
 * and never comes back over the surface the trigger opened.
 *
 * ⚠ **Why a trigger needs this at all.** Every such trigger here is a Radix label `Tooltip`
 * around a NON-modal menu or popover trigger. A non-modal surface leaves pointer events on the
 * page, so a pointer that left the open trigger and came back fired the tooltip's `pointermove`
 * again, and 500ms later the trigger's own name sat over the surface it had opened. On the two
 * composer chips that was the popover's and the menu's bottom edge, both of which open upward,
 * and Escape then closed the hint instead of the menu. Measured in Chromium on the Tools chip,
 * the model chip, the composer's `+` and a row's `⋯` (`surfaceTriggerHint.browser.test.tsx`).
 *
 * ⚠ **The open is refused AT THE TRIGGER, never in `onOpenChange`.** Radix tells the shared
 * provider that a tooltip opened before it calls `onOpenChange`, so a root that ignores the
 * `true` leaves the provider's skip flag set and the next tooltip anywhere opens with no delay.
 * A default-prevented `pointermove` is skipped by Radix's own handler, so the provider never
 * hears of it (`PreviewTip` in `artifacts/PreviewBar.tsx` refuses the same way, for the same
 * reason).
 *
 * ⚠ **A focus is refused at the trigger too, for the same reason.** Radix opens a tooltip on any
 * focus a press did not start, and a closing menu or popover hands focus back to its trigger by
 * script: Escape on the model chip's menu, the `+` menu or a row's `⋯`. `Tooltip`'s Tab gate
 * (Q2-56) declines that open, but inside `onOpenChange`, after Radix has told the provider a
 * tooltip opened and with no close to follow, so the next label tooltip anywhere opened in 12ms
 * instead of 500 (measured in Chromium, `surfaceTriggerHint.browser.test.tsx`). Here the focus
 * event is cancelled before Radix's own handler reads it, which `Tooltip` cannot do for every
 * trigger (a tab or a roving item acts on its focus) but these triggers can: a menu or popover
 * trigger does nothing on focus, so the cancelled event skips only the tooltip. A Tab focus
 * still shows the hint, unless the surface is open.
 *
 * A press also holds the hint shut until the pointer leaves the trigger, which is what Radix
 * does for a plain button: a click that CLOSES the surface does not hand the trigger its name
 * back under a pointer that has not moved off it.
 *
 * ⚠ **A hint that is already showing is closed THROUGH Radix, never by masking `open`.** A surface
 * can open while the hint shows without a press on the trigger: Enter, Space or ArrowDown on a
 * DropdownMenu trigger Tab reached (the menu opens on keydown and no click follows), or the app
 * opening the Tools popover itself. Radix tells the provider a tooltip CLOSED only from its own
 * state change, and only when the new value differs from the `open` prop; a parent that flips
 * the prop to `false` first leaves Radix nothing to change, so the provider's skip flag, cleared
 * when the hint opened, stayed cleared and the next label tooltip anywhere opened with no delay
 * (measured in Chromium on the model chip, the `+` and a row's `⋯`). So the prop stays as the hint
 * left it, and when the surface opens over a showing hint the hint is asked to close the way
 * Radix closes a tooltip another one replaced (`closeShowingTooltips`).
 *
 * The root is CONTROLLED from its first render to its last (#50: a root that flips between
 * controlled and uncontrolled is what once left a tooltip stuck open).
 *
 * ```tsx
 * const hint = useSurfaceTriggerHint(menuOpen);
 * <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
 *   <Tooltip {...hint.tooltip}>
 *     <TooltipTrigger asChild {...hint.trigger}>
 *       <DropdownMenuTrigger asChild>…</DropdownMenuTrigger>
 * ```
 */
export function useSurfaceTriggerHint(surfaceOpen: boolean) {
  const [open, setOpen] = React.useState(false);
  const surfaceOpenRef = React.useRef(surfaceOpen);
  surfaceOpenRef.current = surfaceOpen;
  const pressed = React.useRef(false);
  // The surface opened while the hint showed: the hint goes for good, so it does not reappear by
  // itself when the surface closes. Before paint, and through Radix, so the provider hears it.
  React.useLayoutEffect(() => {
    if (surfaceOpen && open) closeShowingTooltips();
  }, [surfaceOpen, open]);

  const refuse = (event: React.PointerEvent) => {
    if (pressed.current || surfaceOpenRef.current) event.preventDefault();
  };

  return {
    tooltip: {
      open,
      onOpenChange: (next: boolean) => setOpen(next && !surfaceOpenRef.current),
    },
    trigger: {
      onPointerDown: () => {
        pressed.current = true;
      },
      onPointerMove: refuse,
      onPointerLeave: () => {
        pressed.current = false;
      },
      // Runs before Radix's focus handler (`Tooltip`'s trigger calls it first), so a cancelled
      // focus never reaches the provider.
      onFocus: (event: React.FocusEvent) => {
        if (surfaceOpenRef.current || !isFocusFromTabKey()) event.preventDefault();
      },
    },
  };
}

/**
 * Radix's own "a tooltip opened" signal (`TOOLTIP_OPEN` in `@radix-ui/react-tooltip`), on which
 * every showing tooltip closes itself through its own state, the one path that tells the shared
 * provider a tooltip closed. It is not exported, so the name is spelled here;
 * `surfaceTriggerHint.test.tsx` fails if a Radix upgrade renames it (the hint would stay over the
 * open surface). Only Radix tooltips listen: an InfoTip or a footer card is a popover and ignores
 * it.
 */
const RADIX_TOOLTIP_OPEN_EVENT = 'tooltip.open';

export function closeShowingTooltips() {
  document.dispatchEvent(new CustomEvent(RADIX_TOOLTIP_OPEN_EVENT));
}
