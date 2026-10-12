import {
  createContext,
  forwardRef,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
  type ComponentProps,
  type ReactElement,
  type ReactNode,
  type SyntheticEvent,
} from 'react';
import { createPortal } from 'react-dom';
import { cn } from '../../utils';
import { Button } from '../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger, isFocusFromTabKey } from '../ui/Tooltip';

/**
 * THE PREVIEW BAR (spec 3.8): one 32px row under the tab strip, the same for
 * every kind of artifact, so the panel speaks with one voice.
 *
 * The bar is rendered by `ArtifactViewer` from the artifact's own metadata —
 * its name and path — BEFORE the content loads, so a tab switch or a slow read
 * never takes it away. What only the content knows (a line count, a table's
 * shape, a Copy, the Preview/Raw switch) the content PORTALS into the bar's two
 * slots with `<PreviewBarMeta>` and `<PreviewBarActions>`. Each preview kind
 * keeps owning its own state; the bar only gives it somewhere to stand.
 *
 *   [ name ] [ meta … ]                                [ actions ]
 *
 * The bar's horizontal padding is the paper inset (`preview-panel.css`), so its
 * first glyph sits on the same left edge as the prose, the code text and the
 * table's first column beneath it.
 */

type PreviewBarSlots = { meta: HTMLElement | null; actions: HTMLElement | null };

type PreviewBarScopeValue = {
  slots: PreviewBarSlots;
  setMetaSlot: (node: HTMLElement | null) => void;
  setActionsSlot: (node: HTMLElement | null) => void;
};

const PreviewBarScopeContext = createContext<PreviewBarScopeValue | null>(null);

/**
 * Owns the bar's slots for one panel. Wrap the bar AND the content in it: the
 * bar registers its slot elements, the content portals into them.
 */
export function PreviewBarScope({ children }: { children: ReactNode }) {
  const [slots, setSlots] = useState<PreviewBarSlots>({ meta: null, actions: null });
  // A callback ref runs in the commit, and a state update there re-renders
  // before the browser paints: the slot is filled in the bar's first frame.
  const setMetaSlot = useCallback(
    (node: HTMLElement | null) =>
      setSlots((current) => (current.meta === node ? current : { ...current, meta: node })),
    []
  );
  const setActionsSlot = useCallback(
    (node: HTMLElement | null) =>
      setSlots((current) => (current.actions === node ? current : { ...current, actions: node })),
    []
  );
  const value = useMemo(
    () => ({ slots, setMetaSlot, setActionsSlot }),
    [slots, setMetaSlot, setActionsSlot]
  );
  return (
    <PreviewBarScopeContext.Provider value={value}>{children}</PreviewBarScopeContext.Provider>
  );
}

/**
 * Wraps every layer of the body, and gives the bar to the layer being shown.
 * A tab switch keeps the outgoing tab's DOM for one 125ms fade, and its
 * `PreviewBarMeta` and `PreviewBarActions` portal into the bar, which is OUTSIDE
 * the layer: without this the bar held the outgoing tab's count, Copy and view
 * switch beside the incoming tab's for those frames (measured: two Copy buttons
 * and two switches at once). A `detached` layer puts nothing into the bar.
 *
 * It is rendered for the current layer too, with `detached` false, so a layer
 * that turns from current to outgoing changes a context value and nothing else:
 * its subtree keeps the same shape, so React does not remount it, and a frame
 * does not reload or a page re-render while it fades.
 */
export function PreviewBarGate({ detached, children }: { detached: boolean; children: ReactNode }) {
  const scope = useContext(PreviewBarScopeContext);
  return (
    <PreviewBarScopeContext.Provider value={detached ? null : scope}>
      {children}
    </PreviewBarScopeContext.Provider>
  );
}

/** The content's metadata (`300 lines`, `500 of 620 rows`), shown in the bar. */
export function PreviewBarMeta({ children }: { children: ReactNode }) {
  const scope = useContext(PreviewBarScopeContext);
  if (!scope?.slots.meta) return null;
  return createPortal(children, scope.slots.meta);
}

/** The content's controls (Copy, the view switch), shown at the bar's right edge. */
export function PreviewBarActions({ children }: { children: ReactNode }) {
  const scope = useContext(PreviewBarScopeContext);
  if (!scope?.slots.actions) return null;
  return createPortal(children, scope.slots.actions);
}

export function PreviewBar({
  name,
  path,
  className,
}: {
  /** The artifact's name: a file's basename, a figure's title. */
  name: string;
  /** The full path or URL, shown on hover. A machine string, so never on the row itself. */
  path?: string;
  className?: string;
}) {
  const scope = useContext(PreviewBarScopeContext);
  return (
    <div data-testid="artifact-status-strip" className={cn('br-preview-bar', className)}>
      <span className="br-preview-bar__name" title={path && path !== name ? path : undefined}>
        {name}
      </span>
      <span ref={scope?.setMetaSlot} className="br-preview-bar__meta" />
      <span ref={scope?.setActionsSlot} className="br-preview-bar__actions" />
    </div>
  );
}

/**
 * Whether the panel is open. `ArtifactViewer` provides its `isOpen`, which turns false the moment
 * Close is pressed and stays false for the exit before the panel unmounts. Every control's
 * tooltip (`PreviewTip`) reads it. True by default, so a control mounted outside a panel (a test)
 * is always present.
 */
const PreviewPanelPresenceContext = createContext(true);
export const PreviewPanelPresence = PreviewPanelPresenceContext.Provider;

/**
 * THE PANEL'S TOOLTIP (V2-R8-4). A label tooltip names a control BEFORE it is used, so it opens
 * for neither of the two moments after:
 *
 *   - The control was pressed. Radix keeps a per-control flag, "a move opened my tooltip", and
 *     the flag is what stops the tooltip coming back after a press until the pointer has left and
 *     returned. It is left unset when the control's earlier moves were ignored (Radix ignores a
 *     move while the pointer is "in transit" to a neighbour's tooltip, which is where a pointer
 *     crossing a header's tooltips arrives), and the shared provider then opens the next move's
 *     tooltip at once for 300ms after a neighbour's tooltip closed. Measured in the panel
 *     harness: Close's tooltip `instant-open` 15ms after the click, while the panel was
 *     leaving, alone at the top right over the space the panel had just left. Here a press
 *     holds the tooltip shut until the pointer leaves the control or the keyboard reaches it.
 *   - The panel is leaving (`PreviewPanelPresence` false). Nothing in it is worth naming now:
 *     a tooltip open when the exit starts closes, and none opens until the panel is open again.
 *
 * An open is refused at the trigger (`preventDefault` on the move or the focus, which Radix
 * honours), so the shared provider is never told a tooltip opened that did not: its skip-delay
 * flag would otherwise stay set and open the next tooltip anywhere with no delay.
 *
 * Takes one trigger element; the tooltip says `content`.
 */
export function PreviewTip({ content, children }: { content: ReactNode; children: ReactElement }) {
  const present = useContext(PreviewPanelPresenceContext);
  const presentRef = useRef(present);
  presentRef.current = present;
  const pressed = useRef(false);
  const [open, setOpen] = useState(false);
  // The exit closes what is open for good, so a panel opened again in the same mount does
  // not bring the label back with it.
  if (!present && open) setOpen(false);

  const refuse = (event: SyntheticEvent) => {
    if (pressed.current || !presentRef.current) event.preventDefault();
  };

  return (
    <Tooltip open={open} onOpenChange={(next) => setOpen(next && presentRef.current)}>
      <TooltipTrigger
        asChild
        onPointerDown={() => {
          pressed.current = true;
        }}
        onPointerMove={refuse}
        onPointerLeave={() => {
          pressed.current = false;
        }}
        onFocus={(event) => {
          // Tab is the person's keyboard reaching the control: a press still held no
          // longer speaks for it.
          if (isFocusFromTabKey()) pressed.current = false;
          if (!presentRef.current) event.preventDefault();
        }}
      >
        {children}
      </TooltipTrigger>
      <TooltipContent side="bottom">{content}</TooltipContent>
    </Tooltip>
  );
}

/**
 * One icon control for the panel: a ghost square `Button` with an app tooltip
 * carrying its name. 32px in the 44px header band (the size of every band's
 * actions), 24px inside the 32px bar.
 */
export const PreviewIconButton = forwardRef<
  HTMLButtonElement,
  Omit<ComponentProps<typeof Button>, 'variant' | 'shape' | 'size'> & {
    label: string;
    /** The tooltip, when it should say more than the name. */
    hint?: string;
    compact?: boolean;
  }
>(function PreviewIconButton({ label, hint, compact = false, className, ...props }, ref) {
  const button = (
    <Button
      ref={ref}
      type="button"
      variant="ghost"
      shape="round"
      size={compact ? 'xs' : 'default'}
      aria-label={label}
      className={cn('br-preview-icon-button no-drag', className)}
      {...props}
    />
  );
  // A disabled button takes no pointer events, so a tooltip on it never opens;
  // the name is still read aloud.
  if (props.disabled) return button;
  return <PreviewTip content={hint ?? label}>{button}</PreviewTip>;
});

/**
 * The panel's one loading look (spec 3.8): nothing for 200ms — most reads land
 * sooner — then three quiet lines on the paper, in the reading column. No word:
 * the bar above already names what is coming. `data-preview-loading` tells the
 * stacked sheet to keep waiting for a real height.
 */
export function PreviewSkeleton() {
  return (
    <div data-preview-loading="" aria-busy="true" className="br-preview-skeleton">
      <span />
      <span />
      <span />
    </div>
  );
}
