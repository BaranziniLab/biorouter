import {
  createContext,
  forwardRef,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ComponentProps,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';
import { cn } from '../../utils';
import { Button } from '../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';

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
  return (
    <Tooltip>
      <TooltipTrigger asChild>{button}</TooltipTrigger>
      <TooltipContent side="bottom">{hint ?? label}</TooltipContent>
    </Tooltip>
  );
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
