import * as React from 'react';
import { cn } from '../utils';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from './ui/dialog';

/**
 * The one dialog shell for Biorouter's app-level modals (Astryx §3.6).
 *
 * Every modal outside `components/ui/` renders through this: it owns the size
 * scale, the section geometry (header / scrolling body / footer), the single
 * close affordance, and the dismissal policy. Before it there were six
 * hand-rolled shells and fifteen distinct `max-w` values; two of those shells
 * (`SetupModal`, `InterruptionHandler`) painted their own scrim and are gone,
 * and `BaseModal` lost its last consumer to this file (and was then deleted).
 *
 * It COMPOSES `components/ui/dialog` rather than replacing it — the Radix
 * primitive still owns the portal, the scrim, the focus trap and the ×. In
 * particular `DialogContent` carries `.biorouter-modal-surface`, whose
 * unlayered `z-index: var(--z-modal)` is the structural floor that keeps a
 * modal above its own scrim (which shares that layer and precedes it, so a
 * dialog opened from a dialog dims the first one: V4-08). A shell that paints its own
 * full-screen overlay has no such floor, which is how a modal once ended up
 * *under* the pointer-eating scrim and read as "the whole app froze". Nothing
 * here sets a `z-*` class, on purpose.
 *
 * If `DialogContent` ever grows `size` / `purpose` props of its own, this file
 * should collapse into them; the scale below is the one to hoist.
 */

/**
 * Four widths, replacing the fifteen that were in use. `full` is deliberately
 * absent: nothing needs it, and it cannot be built correctly from here because
 * `.biorouter-modal-surface` sets the corner radius from an unlayered rule that
 * no `rounded-none` utility can beat.
 */
export const MODAL_SIZE = {
  /** S — confirmations and single-decision notices. */
  sm: 'sm:max-w-[400px]',
  /** M — forms. */
  md: 'sm:max-w-[480px]',
  /** L — palettes, reports, anything with a list. */
  lg: 'sm:max-w-[640px]',
  /**
   * XL (800px) — editors and side-by-side content (spec 2.6). The width is ALSO authored on
   * `[data-size='xl']` in main.css, because this arbitrary utility is new and a newly written
   * class can fail to generate under `BIOROUTER_NO_HMR` (CLAUDE.md, "Desktop shell geometry").
   */
  xl: 'sm:max-w-[800px]',
} as const;

export type ModalSize = keyof typeof MODAL_SIZE;

/**
 * Astryx's `purpose` axis, taken wholesale because Biorouter had the bug it
 * prevents — a half-filled parameter form that a stray backdrop click threw
 * away.
 *
 * - `info` — dismisses on Escape, on backdrop click, and via the ×.
 * - `form` — Escape and × still leave; a backdrop click does NOT, so typed
 *   input survives a misclick.
 * - `required` — must be answered. No ×, no Escape, no backdrop. Use it for
 *   the transient "work in flight" state too (`purpose={busy ? 'required' :
 *   'form'}`), so a dismissal cannot orphan an install.
 */
export type ModalPurpose = 'info' | 'form' | 'required';

/**
 * Where the dialog sits vertically.
 *
 * - `top` (the default, app-wide since the Codex simplicity redesign: Crew is the reference
 *   design, coordinator ruling 1): its top edge is pinned at `max(10vh, 48px)` and it grows
 *   downward only. A dialog whose height changes while it is open — a tab switch, a result
 *   replacing a form, an error appearing — otherwise jumps up and down under the pointer (QA T-30).
 * - `center`: centred on the window by the primitive's `top: 50%` plus a -50% Y translate, so it
 *   re-centres whenever its height changes. A dialog that reads wrong pinned to the top (a short,
 *   fixed-height notice) passes it explicitly, with a one-line reason at the call site.
 *
 * Authored as an inline style rather than utilities: an inline declaration always beats the
 * primitive's `top-[50%] translate-y-[-50%]`, and a newly written arbitrary utility can silently
 * fail to generate (see `CLAUDE.md`, "Desktop shell geometry").
 */
export type ModalAnchor = 'center' | 'top';

/** The `top` anchor's geometry. The max-height keeps the bottom edge inside the window. */
export const MODAL_ANCHOR_TOP_STYLE: React.CSSProperties = {
  top: 'max(10vh, 48px)',
  translate: '-50% 0',
  maxHeight: 'min(85vh, calc(100vh - max(10vh, 48px) - 16px))',
};

/**
 * Defaults a surface that mounts many dialogs gives every `ModalShell` inside it, so the dialogs
 * need not each repeat them — and so a dialog that area does not own still gets them. A prop on
 * the shell wins over the default, and the default wins over the shell's own (`anchor: 'top'`,
 * `headerRule: true`, which are what Crew's `CrewDialogs` provides too, now that the shell
 * itself takes them). Crew's remaining difference is a close handler that keeps Radix from moving
 * focus, because Crew returns focus to the opener itself.
 */
export interface ModalShellDefaults {
  anchor?: ModalAnchor;
  onCloseAutoFocus?: (event: Event) => void;
  /**
   * Added to every content's class list, before the shell's own `className`. A dialog portals to
   * `<body>`, outside the surface that mounts it, so an area's stylesheet reaches its dialogs only
   * through a class on the dialog itself (Crew: `crew-dialog`, QA Q2-25).
   */
  className?: string;
  /** Draw the header's hairline even where the body does not scroll (QA Q2-26). */
  headerRule?: boolean;
}

export const ModalShellDefaultsContext = React.createContext<ModalShellDefaults>({});

export interface ModalShellProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  size?: ModalSize;
  purpose?: ModalPurpose;
  title: React.ReactNode;
  /** Status ink for the title (danger/warning). Layout never changes with it. */
  titleClassName?: string;
  /**
   * An `InfoTip` beside the title. It sits OUTSIDE the heading: the dialog is named by the heading
   * (`aria-labelledby`), and an InfoTip inside it would add its "About …" name and its hidden help
   * to the dialog's name (V4-R4-2, "Report a problem").
   */
  titleInfo?: React.ReactNode;
  /** Optional 12px line under the title. */
  subtitle?: React.ReactNode;
  /** Right-aligned action row. */
  footer?: React.ReactNode;
  children?: React.ReactNode;
  /**
   * The body scrolls internally instead of growing the dialog, and the
   * header/footer gain their hairlines so the scroll edge is legible.
   */
  scrollBody?: boolean;
  className?: string;
  bodyClassName?: string;
  /** Vertical placement; `top` unless this prop or a `ModalShellDefaultsContext` says otherwise. */
  anchor?: ModalAnchor;
  /**
   * Radix's close-focus hook, for a dialog opened without a `Dialog.Trigger` that returns focus
   * itself: call `event.preventDefault()` to keep Radix from moving focus.
   */
  onCloseAutoFocus?: (event: Event) => void;
  /**
   * Radix's open-focus hook, for a dialog whose first tabbable is a poor place for focus (a form
   * of Selects, where a closed trigger would wear the focus surface after a mouse open): call
   * `event.preventDefault()` and focus the dialog (`event.currentTarget`) or a field yourself.
   * `DialogContent` records the opener before calling it, so focus still returns on close.
   */
  onOpenAutoFocus?: (event: Event) => void;
  /**
   * The id of the body node that describes the dialog, for a dialog whose body is a message rather
   * than a form (QA Q2-28). Used only without a `subtitle`, which is the description when present.
   */
  describedBy?: string;
  /**
   * The header's hairline, drawn without a scrolling body too. On unless this prop or a
   * `ModalShellDefaultsContext` says otherwise.
   */
  headerRule?: boolean;
}

export function ModalShell({
  open,
  onOpenChange,
  size = 'md',
  purpose = 'info',
  title,
  titleClassName,
  titleInfo,
  subtitle,
  footer,
  children,
  scrollBody = false,
  className,
  bodyClassName,
  anchor,
  onCloseAutoFocus,
  onOpenAutoFocus,
  describedBy,
  headerRule,
}: ModalShellProps) {
  const dismissible = purpose !== 'required';
  const defaults = React.useContext(ModalShellDefaultsContext);
  const placement = anchor ?? defaults.anchor ?? 'top';
  const closeAutoFocus = onCloseAutoFocus ?? defaults.onCloseAutoFocus;
  const rule = scrollBody || (headerRule ?? defaults.headerRule ?? true);
  // A subtitle is the description (Radix links it). Without one, a message body can name its own
  // node; otherwise the attribute is dropped rather than dangled at nothing.
  const bodyDescribes = !subtitle && Boolean(describedBy);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        dismissible={dismissible}
        // `dismissible` already blocks Escape and the backdrop for `required`.
        // This is the one extra rule the purpose axis adds: a form keeps its
        // Escape route but ignores the backdrop.
        onPointerDownOutside={(event) => {
          if (purpose === 'form') event.preventDefault();
        }}
        // Radix warns when a dialog has no description. A subtitle supplies one
        // (and must NOT be overridden, or the aria linkage breaks); without a
        // subtitle we opt out explicitly rather than leave a console warning.
        {...(subtitle ? {} : { 'aria-describedby': bodyDescribes ? describedBy : undefined })}
        {...(closeAutoFocus ? { onCloseAutoFocus: closeAutoFocus } : {})}
        {...(onOpenAutoFocus ? { onOpenAutoFocus } : {})}
        data-anchor={placement === 'top' ? 'top' : undefined}
        data-size={size}
        style={placement === 'top' ? MODAL_ANCHOR_TOP_STYLE : undefined}
        className={cn(
          'flex max-h-[85vh] flex-col gap-0 overflow-hidden p-0',
          MODAL_SIZE[size],
          defaults.className,
          className
        )}
      >
        <div
          className={cn(
            // The × is 32px inset 16px, so the title column stops at pr-12 and
            // a long title can never slide under it.
            'flex-none px-4 pt-4 pb-3',
            dismissible && 'pr-12',
            rule && 'border-b border-border-subtle'
          )}
        >
          {titleInfo != null ? (
            <div className="flex min-w-0 items-center">
              <DialogTitle className={cn('min-w-0 [overflow-wrap:anywhere]', titleClassName)}>
                {title}
              </DialogTitle>
              {titleInfo}
            </div>
          ) : (
            <DialogTitle className={cn('min-w-0 [overflow-wrap:anywhere]', titleClassName)}>
              {title}
            </DialogTitle>
          )}
          {subtitle ? (
            <DialogDescription className="mt-1 text-supporting">{subtitle}</DialogDescription>
          ) : bodyDescribes ? (
            // Radix checks that ITS description id exists whenever `aria-describedby` is set, and
            // warns otherwise. The body's node is the description; this empty, hidden anchor only
            // answers that check and is referenced by nothing.
            <DialogDescription hidden />
          ) : null}
        </div>

        {children != null && (
          <div
            className={cn(
              'min-w-0 px-4',
              // The header's pb-3 and the footer's pt-3 are the body's vertical
              // gutters; the body adds none of its own, so a scrolled body runs
              // to the hairline instead of stranding a dead band above it.
              scrollBody ? 'min-h-0 flex-1 overflow-y-auto' : 'flex-none',
              // Under a hairline the body needs its own top gutter; a scrolling body owns its own.
              rule && !scrollBody && 'pt-3',
              bodyClassName
            )}
          >
            {children}
          </div>
        )}

        {footer != null && (
          <div
            className={cn(
              'flex-none flex flex-wrap items-center justify-end gap-2 px-4 pt-3 pb-4',
              scrollBody && 'border-t border-border-subtle'
            )}
          >
            {footer}
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
