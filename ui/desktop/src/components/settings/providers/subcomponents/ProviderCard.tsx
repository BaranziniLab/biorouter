import { useMemo } from 'react';
import { ChevronDown } from '../../../icons/app-icons';
import DefaultCardButtons from './buttons/DefaultCardButtons';
import { ProviderDetails, ProviderMetadata } from '../../../../api';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../../ui/Tooltip';
import { StatusDot } from '../../../ui/status-dot';
import { catalogCopy } from '../copy';
import { InlineCodeText } from './InlineCodeText';
import '../providers.css';

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
  children,
}: ProviderCardProps) {
  const providerMetadata: ProviderMetadata | null = provider?.metadata || null;
  const metadata = useMemo(() => providerMetadata, [providerMetadata]);

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
        <Tooltip>
          {/* Focusable, so a keyboard user can open the tooltip that holds the
              rest of a truncated description: a Radix trigger opens on focus
              as well as hover, but only if it can take focus. */}
          <TooltipTrigger asChild>
            <p
              tabIndex={0}
              data-testid={`provider-description-${slug}`}
              className="-mx-1 -my-0.5 cursor-default truncate rounded-element px-1 py-0.5 text-supporting text-text-muted biorouter-focus-surface"
            >
              <InlineCodeText text={metadata.description} />
            </p>
          </TooltipTrigger>
          <TooltipContent side="bottom" className="max-w-72 text-wrap">
            <InlineCodeText text={metadata.description} />
          </TooltipContent>
        </Tooltip>
      )}
    </div>
  );

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
  const configured = provider.is_configured ? (
    <span className="br-provider-configured br-row-actions-meta text-supporting text-text-muted">
      <StatusDot tone="success" />
      {catalogCopy.configured}
    </span>
  ) : null;
  const action = (
    <div data-testid={`provider-actions-${slug}`} className="br-row-actions flex items-center">
      <DefaultCardButtons
        provider={provider}
        onConfigure={onConfigure}
        onLaunch={onLaunch}
        isOnboardingPage={isOnboarding}
      />
    </div>
  );
  const actions = (
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
            onClick={onToggle}
            data-testid={`provider-row-toggle-${slug}`}
            className="br-provider-row-toggle cursor-pointer"
          >
            {identity}
          </button>
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

  // The row's click duplicates its action button for a mouse user: Configure, or Launch on a
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
      {actions}
    </div>
  );
};
