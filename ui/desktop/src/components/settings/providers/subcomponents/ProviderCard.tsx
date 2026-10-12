import { useId, useLayoutEffect, useMemo, useRef, useState, type RefObject } from 'react';
import { ChevronDown } from '../../../icons/app-icons';
import DefaultCardButtons from './buttons/DefaultCardButtons';
import { ProviderDetails, ProviderMetadata } from '../../../../api';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../../ui/Tooltip';
import { StatusDot } from '../../../ui/status-dot';
import { catalogCopy, providerRowLine } from '../copy';
import '../providers.css';

/**
 * Whether a one-line element is cut off: its text is wider than its box. Measured after layout and
 * again whenever the box resizes, so a description that fits in a wide window and is cut in a
 * narrow one gains and loses its Tab stop with the window.
 */
function useIsTruncated(ref: RefObject<HTMLElement | null>, text: string): boolean {
  const [truncated, setTruncated] = useState(false);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const measure = () => setTruncated(element.scrollWidth > element.clientWidth + 1);
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref, text]);
  return truncated;
}

type ProviderCardProps = {
  provider: ProviderDetails;
  onConfigure: () => void;
  onLaunch: () => void;
  isOnboarding: boolean;
  /**
   * The row opens in place instead of acting on click — the catalog's accordion.
   * When set, {@link ProviderCardProps.children} is the panel it opens.
   */
  expandable?: boolean;
  expanded?: boolean;
  onToggle?: () => void;
  /** A live status line beside the name (the coding agents' auth pill). */
  statusSlot?: React.ReactNode;
  /**
   * The row's second line, when the daemon's description is not a line: the coding agents'
   * (`agentCopy.rowSummary`) and the rows in `providerRowSummary` (the two Versa gateways, GitHub
   * Copilot, Xiaomi MiMo). The description stays the row's accessible description.
   */
  summary?: string;
  children?: React.ReactNode;
};

/**
 * One row of the provider catalog: the shared settings row, a name over a one-line description
 * on the left, "Configured" as a success dot and a word, and one action on the right.
 *
 * ⚠ **One row definition, two behaviours.** An expandable row and a
 * click-to-configure row are the same header markup: the same name and one-line
 * description, the same "Configured" status, the same hover actions. They sit in the
 * same list and any divergence reads as two different kinds of thing. Only what the click
 * *does* differs. There is no initial tile: a Codex-style row is text.
 *
 * ⚠ **A row's actions are revealed the same way on every surface** (principle 6, checklist 8):
 * "Configured" at rest in the cell the action takes over on hover or focus, always shown without
 * a hover pointer. First run used to keep its button permanently on screen, which made it the one
 * list in the app with always-visible row actions on a hover-capable display (V4-29). Its rows now
 * act on a click, as the Settings ones do, so the button is a shortcut for the mouse and the
 * keyboard path (opacity only, so it stays in the tab order) and never the only way in.
 *
 * ⚠ **The disclosure chevron follows the name** ("Claude Code ⌄", as Codex draws a group), not
 * the row's left edge, so every title in the list starts on the same x (V4-26).
 *
 * ⚠ **No state-dependent shading.** An unconfigured provider used to render at
 * `opacity-50` during onboarding, which made the rows a user is there to act on
 * the faintest thing on the screen. Configured state is said in words
 * ("Configured", with a dot) rather than by dimming everything that is not.
 *
 * ⚠ **The rows stay siblings.** `.biorouter-settings-row:last-child` is what drops the last
 * hairline, so an expandable row and its panel are two siblings in the list, not one wrapper
 * (a wrapper made every row its parent's last child and erased every hairline).
 */
export const ProviderCard = function ProviderCard({
  provider,
  onConfigure,
  onLaunch,
  isOnboarding,
  expandable = false,
  expanded = false,
  onToggle,
  statusSlot,
  summary,
  children,
}: ProviderCardProps) {
  const providerMetadata: ProviderMetadata | null = provider?.metadata || null;
  const metadata = useMemo(() => providerMetadata, [providerMetadata]);
  const descriptionRef = useRef<HTMLParagraphElement>(null);
  const descriptionId = useId();
  const fullDescriptionId = useId();
  const descriptionTruncated = useIsTruncated(
    descriptionRef,
    summary ?? metadata?.description ?? ''
  );
  // The line's tooltip opens only while the line is cut off (vision FR-R2-7): over a line that
  // fits, it only said the same thing again, at length, over the next row.
  const [descriptionTipOpen, setDescriptionTipOpen] = useState(false);

  if (!metadata) {
    return <div>ProviderCard error: No metadata provided</div>;
  }

  const displayName = metadata.display_name || provider?.name || 'Unknown Provider';
  const slug = provider.name.toLowerCase();

  /** Name + description: everything that is pure identification. */
  const identity = (
    <div className="min-w-0 flex-1 text-left">
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5">
        <p className="truncate text-label text-text-default">{displayName}</p>
        {expandable ? (
          <ChevronDown
            className="br-provider-chevron size-3.5"
            data-expanded={expanded ? 'true' : 'false'}
            aria-hidden
          />
        ) : null}
        {statusSlot}
      </div>
      {metadata.description && (
        <Tooltip
          open={descriptionTruncated && descriptionTipOpen}
          onOpenChange={setDescriptionTipOpen}
        >
          {/* A Tab stop ONLY while it is cut off, so a keyboard user can open the tooltip that
              holds the rest (a Radix trigger opens on focus as well as hover, but only if it can
              take focus). Every description used to be one, truncated or not, so the walk went
              row, description, action for every provider: about 27 extra stops on Public (vision
              FR-R1-9). The row's action names the full text as its description either way, so a
              screen reader hears it without the stop. The 6px and 4px of room are for the focus
              surface, which hugged the glyphs; the negative margins keep the layout. */}
          <TooltipTrigger asChild>
            <p
              ref={descriptionRef}
              id={descriptionId}
              tabIndex={descriptionTruncated ? 0 : undefined}
              data-testid={`provider-description-${slug}`}
              className="-mx-1.5 -my-1 cursor-default truncate rounded-element px-1.5 py-1 text-supporting text-text-muted biorouter-focus-surface"
            >
              {summary ?? providerRowLine(metadata.description)}
            </p>
          </TooltipTrigger>
          {/* What is cut off, and only that: the line's own text, in the tooltip's one face (no
              mono run: a tooltip is one family, and since FR-R3-4 so is the line). An agent's short
              line was answered with the daemon's three sentences, `claude` in mono, covering the
              Codex row (FR-R2-7); that sentence is the row's accessible description instead,
              below. */}
          <TooltipContent side="bottom" className="max-w-72 text-wrap">
            {summary ?? metadata.description.replace(/`/g, '')}
          </TooltipContent>
        </Tooltip>
      )}
    </div>
  );

  /**
   * The daemon's own sentence, for assistive tech, when the line shows a short summary in its
   * place (the coding agents', and the rows in `providerRowSummary`). The row's action, or its
   * toggle, is DESCRIBED by it.
   *
   * ⚠ **It sits beside the toggle, never inside it** (vision FR-R4-3, principle 12). A button's
   * accessible name is computed from its content, sr-only text included, so inside the toggle it
   * made Claude Code's name about 200 characters ("Claude Code Your Claude subscription · no API
   * key Uses the Claude subscription you are already signed in to …"), and the description then
   * read the same sentences again. Beside it, the name is the title and the short line, as on
   * every other row, and `aria-describedby` still reaches it by id.
   */
  const fullDescription =
    metadata.description && summary ? (
      <span id={fullDescriptionId} className="sr-only">
        {metadata.description.replace(/`/g, '')}
      </span>
    ) : null;

  /**
   * The configured status and the action button, in one cell: Crew's `meta` recipe
   * (`.br-row-actions-slot`). "Configured" shows at rest on the right edge and the action takes its
   * place while the row is hovered or holds focus, so the edge never moves and the button stays in
   * the tab order (opacity only).
   *
   * ⚠ `.br-row-actions` is the shared reveal: opacity only, on the row's hover AND focus-within,
   * and always shown without a hover pointer. Hover alone left a keyboard user tabbing onto a
   * Configure button, and its focus ring, at opacity 0 (W2-PRV-4). This button is the row's
   * keyboard path: the row's own click is a mouse convenience that does the same thing.
   */
  /**
   * ⚠ **An expandable row on first run has no row action** (vision FR-R1-5, principle 5: one job,
   * one control). Its panel holds the primary for the same decision ("Use Claude Code", "Download
   * and start", "Connect"), so a hover action beside it was a second control for one choice. The
   * row's resting status stays, and stays visible under the pointer, because nothing takes its
   * cell. Settings keeps the action on its expandable rows (an agent's Configure opens its command
   * path), where the panel has no primary that does the same thing.
   */
  const panelHoldsTheAction = isOnboarding && expandable;
  const configured = provider.is_configured ? (
    <span
      className={`br-provider-configured text-supporting text-text-muted${
        panelHoldsTheAction ? '' : ' br-row-actions-meta'
      }`}
    >
      <StatusDot tone="success" />
      {catalogCopy.configured}
    </span>
  ) : null;
  // The full description, as the row's action's (or toggle's) accessible description.
  const fullDescriptionRef = metadata.description
    ? summary
      ? fullDescriptionId
      : descriptionId
    : undefined;
  const action = (
    <div data-testid={`provider-actions-${slug}`} className="br-row-actions flex items-center">
      <DefaultCardButtons
        provider={provider}
        onConfigure={onConfigure}
        onLaunch={onLaunch}
        isOnboardingPage={isOnboarding}
        describedBy={fullDescriptionRef}
      />
    </div>
  );
  const actions = panelHoldsTheAction ? (
    configured
  ) : (
    <div
      className="br-row-actions-slot"
      data-slot="row-actions-slot"
      data-has-meta={configured ? 'true' : undefined}
    >
      {configured}
      {action}
    </div>
  );

  if (expandable) {
    return (
      <>
        <div
          data-testid={`provider-card-${slug}`}
          className="biorouter-settings-row br-setting-row"
        >
          {/*
            A real <button>, so the row is reachable by keyboard and announces
            its own state. The action buttons stay OUTSIDE it: a <button> nested
            in a <button> is invalid markup that browsers silently reparent,
            which detaches the inner handler.
          */}
          <button
            type="button"
            aria-expanded={expanded}
            // The visible line is already part of the button's name; a short summary's full
            // sentence is not, so it is the button's description.
            aria-describedby={summary && metadata.description ? fullDescriptionId : undefined}
            onClick={onToggle}
            data-testid={`provider-row-toggle-${slug}`}
            className="br-provider-row-toggle cursor-pointer"
          >
            {identity}
          </button>
          {fullDescription}
          {actions}
        </div>
        {expanded && (
          <div className="br-provider-panel" data-testid={`provider-row-panel-${slug}`}>
            {children}
          </div>
        )}
      </>
    );
  }

  // The row's click duplicates its action button for a mouse user: Configure, or Use on a
  // configured provider on first run (what the button says, via `DefaultCardButtons`). It is
  // deliberately not a second focusable control: a role="button" wrapping a real <button> is nested
  // interactive markup, and the button, revealed on focus above, reaches the same place by keyboard.
  return (
    <div
      data-testid={`provider-card-${slug}`}
      onClick={isOnboarding && provider.is_configured ? onLaunch : onConfigure}
      className="biorouter-settings-row br-setting-row cursor-pointer"
    >
      {identity}
      {fullDescription}
      {actions}
    </div>
  );
};
