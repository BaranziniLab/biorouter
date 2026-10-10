import React from 'react';
import { ArrowLeft, Info } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { ReadableContent } from './ReadableContent';

export type PageHeaderProps = {
  /** The page title: the only `<h1>` on the page, drawn at `text-label` (14px). */
  title: string;
  /**
   * What the page is for, as hover and focus help (an InfoTip beside the title). Never a
   * visible paragraph: the band has one line. Plain text of at most two sentences.
   */
  info?: React.ReactNode;
  /** A count or a state beside the title, in `text-supporting` muted tabular figures. */
  adornment?: React.ReactNode;
  /**
   * The page's actions, at the band's trailing edge: ghost round 32px icon buttons
   * (`PageHeaderAction`), at most ONE primary `Button` at the default size, and an optional
   * `FilterInput`. Every control here is `no-drag`; the band authors that rule.
   */
  actions?: React.ReactNode;
  /** Underline tabs rendered inline after the title, filling the band's height. */
  tabs?: React.ReactNode;
  /** A drill-in's way back. Renders a ghost round 32px back button before the title. */
  onBack?: () => void;
  /** The back button's accessible name and tooltip. */
  backLabel?: string;
  /** @deprecated Use `info`. Rendered as `info`, so owners can migrate one file at a time. */
  description?: React.ReactNode;
  /** @deprecated Use `adornment`. */
  titleAdornment?: React.ReactNode;
  /**
   * @deprecated The band has one line. A control that changes what a list SHOWS (Chat
   * history's "Show subagent runs") is rendered in a row under the band until its owner moves
   * it into the band. New code should not use it.
   */
  children?: React.ReactNode;
};

/**
 * The one page header for every top-level view: Workflows, the Scheduler, Built apps,
 * Extensions, Skills, Chat history, Settings, the not-found page, and the drill-ins that pass
 * `onBack`.
 *
 * **It is a 44px band**, the Crew channel header's geometry (`crew/channel/ChannelHeader.tsx`):
 * `--chrome-height`, the `--sidebar` ground, a bottom hairline, a 14px title on the left with
 * its help in an InfoTip, and the actions on the right. The band's hairline meets the sidebar's
 * titlebar band and the chat header at y=44: one continuous top edge. Every rule that makes it
 * so is authored in `main.css` (`.biorouter-page-header[data-band]`), not written as Tailwind
 * strings, because a newly written utility can silently fail to generate under
 * `BIOROUTER_NO_HMR`.
 *
 * ⚠ **This reverses the 2026-09-07 "actions on their own line" decision** (operator, recorded
 * as reversed on 2026-10-09 in `docs/design/astryx-adoption/astryx-ui-adoption-design.md`
 * §4.2). Its argument was that a title row with controls must truncate the title; the band
 * answers it, because the title is 14px and the measure is the pane, not 760px.
 *
 * ⚠ **The `<header>` declares no app-region; its inner bar does.** The bar is the window-drag
 * rect (the band replaces the 32px drag strip on its route, `routeOwnsTopBand` in
 * `AppLayout.tsx`). With the app sidebar collapsed the floating titlebar controls sit over the
 * band's left end, and Electron folds drag rects in TREE order, so a band that dragged from x=0
 * would re-cover those earlier `no-drag` controls and kill them (issue #74). The bar therefore
 * carries the titlebar reserve as a MARGIN, outside its own border box, exactly as the chat
 * header's drag rect does (`BaseChat.tsx`). The hairline and the ground stay on the header, so
 * they still run edge to edge.
 *
 * The band never sits inside `ReadableContent`; only the view's body does.
 */
export function PageHeader({
  title,
  info,
  adornment,
  actions,
  tabs,
  onBack,
  backLabel = 'Back',
  description,
  titleAdornment,
  children,
}: PageHeaderProps) {
  const help = info ?? description;
  const trailing = adornment ?? titleAdornment;
  return (
    <>
      <header className="biorouter-page-header" data-band="" data-testid="page-header">
        <div className="biorouter-page-header-bar">
          {onBack && <PageHeaderAction icon={ArrowLeft} label={backLabel} onClick={onBack} />}
          <div className="biorouter-page-header-title">
            <h1 className="text-label min-w-0 truncate">{title}</h1>
            {help ? <BandInfo label={title}>{help}</BandInfo> : null}
          </div>
          {trailing ? (
            <div className="biorouter-page-header-adornment text-supporting text-text-muted tabular-nums">
              {trailing}
            </div>
          ) : null}
          {tabs ? <div className="biorouter-page-header-tabs">{tabs}</div> : null}
          {actions ? <div className="biorouter-page-header-actions">{actions}</div> : null}
        </div>
      </header>
      {children ? (
        <div className="biorouter-page-subband">
          <ReadableContent size="chat" className="px-6 pt-2">
            {children}
          </ReadableContent>
        </div>
      ) : null}
    </>
  );
}

type IconComponent = React.ComponentType<React.SVGProps<SVGSVGElement>>;

export type PageHeaderActionProps = Omit<
  React.ComponentProps<typeof Button>,
  'children' | 'variant' | 'shape' | 'size' | 'title'
> & {
  icon: IconComponent;
  /** The accessible name and the tooltip. Never `title=` (AppTooltipLayer renames from it). */
  label: string;
  /** Optional tooltip text when it should say more than the name ("Filter · ⌘F"). */
  tooltip?: string;
};

/**
 * One band action: a ghost round 32px icon button with a `Tooltip` and an `aria-label`, the
 * Crew channel header's right-cluster control. Exported so every view's band uses one shape.
 */
export const PageHeaderAction = React.forwardRef<HTMLButtonElement, PageHeaderActionProps>(
  ({ icon: Icon, label, tooltip, ...props }, ref) => (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button ref={ref} variant="ghost" shape="round" aria-label={label} {...props}>
          <Icon />
        </Button>
      </TooltipTrigger>
      <TooltipContent side="bottom">{tooltip ?? label}</TooltipContent>
    </Tooltip>
  )
);
PageHeaderAction.displayName = 'PageHeaderAction';

/**
 * The title's help. A focusable `About {title}` button holding a 14px Info glyph, opening the
 * help on hover and on Tab focus, with the same text always present in a visually hidden node
 * the button names through `aria-describedby`, so a screen reader hears it without a hover.
 *
 * Interim: replaced by WS-PRIMITIVES' `ui/info-tip.tsx` once that contract lands; the props
 * and the accessible shape (name, description) are the InfoTip's.
 */
function BandInfo({ label, children }: { label: string; children: React.ReactNode }) {
  const id = React.useId();
  return (
    <>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            className="biorouter-page-header-info"
            aria-label={`About ${label}`}
            aria-describedby={id}
          >
            <Info aria-hidden="true" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="bottom" align="start" className="biorouter-page-header-info-text">
          {children}
        </TooltipContent>
      </Tooltip>
      <span id={id} className="sr-only">
        {children}
      </span>
    </>
  );
}
