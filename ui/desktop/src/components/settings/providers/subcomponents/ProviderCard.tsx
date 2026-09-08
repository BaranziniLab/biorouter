import { useMemo } from 'react';
import { Check, ChevronDown } from '../../../icons/app-icons';
import DefaultCardButtons from './buttons/DefaultCardButtons';
import { ProviderDetails, ProviderMetadata } from '../../../../api';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../../ui/Tooltip';

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
 * One row of the provider catalog.
 *
 * ⚠ **One row definition, two behaviours.** An expandable row and a
 * click-to-configure row are the same header markup — the same avatar, the same
 * name and one-line description, the same "Configured" check, the same hover
 * actions — because they sit in the same list and any divergence reads as two
 * different kinds of thing. Only what the click *does* differs.
 *
 * ⚠ **No state-dependent shading.** An unconfigured provider used to render at
 * `opacity-50` during onboarding, which made the rows a user is there to act on
 * the faintest thing on the screen. Configured state is said in words
 * ("Configured", with a check) rather than by dimming everything that is not.
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
  const initial = displayName[0].toUpperCase();
  const slug = provider.name.toLowerCase();

  /** Avatar + name + description: everything that is pure identification. */
  const identity = (
    <>
      <div className="w-8 h-8 rounded-element bg-background-medium flex items-center justify-center flex-shrink-0 text-sm font-semibold text-text-muted select-none">
        {initial}
      </div>
      <div className="flex-1 min-w-0 text-left">
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5">
          <p className="text-sm font-medium text-text-default truncate">{displayName}</p>
          {statusSlot}
        </div>
        {metadata.description && (
          <Tooltip>
            <TooltipTrigger asChild>
              <p className="text-xs text-text-muted mt-0.5 truncate cursor-default">
                {metadata.description}
              </p>
            </TooltipTrigger>
            <TooltipContent side="bottom" className="max-w-72 text-wrap">
              {metadata.description}
            </TooltipContent>
          </Tooltip>
        )}
      </div>
    </>
  );

  /** The configured check and the action buttons. */
  const actions = (
    <div className="flex items-center gap-3 flex-shrink-0">
      {provider.is_configured && (
        <span className="flex items-center gap-1 text-xs text-text-success font-medium">
          <Check className="w-3 h-3" />
          Configured
        </span>
      )}
      <div
        className={
          !isOnboarding && !expandable
            ? 'opacity-0 group-hover:opacity-100 transition-opacity duration-150'
            : ''
        }
      >
        <DefaultCardButtons
          provider={provider}
          onConfigure={onConfigure}
          onLaunch={onLaunch}
          isOnboardingPage={isOnboarding}
        />
      </div>
    </div>
  );

  if (expandable) {
    return (
      <div data-testid={`provider-card-${slug}`} className="min-w-0">
        <div className="flex items-center gap-3 py-3 px-4 rounded-container transition-colors group tint-interactive">
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
            className="flex flex-1 min-w-0 items-center gap-3 cursor-pointer text-left"
          >
            <ChevronDown
              className={`w-3.5 h-3.5 flex-shrink-0 text-text-muted transition-transform duration-[var(--motion-fast)] ${
                expanded ? '' : '-rotate-90'
              }`}
              aria-hidden
            />
            {identity}
          </button>
          {actions}
        </div>
        {expanded && (
          <div className="px-4 pb-4" data-testid={`provider-row-panel-${slug}`}>
            {children}
          </div>
        )}
      </div>
    );
  }

  return (
    <div
      data-testid={`provider-card-${slug}`}
      onClick={!isOnboarding ? onConfigure : undefined}
      className={[
        'flex items-center gap-3 py-3 px-4 rounded-container',
        'transition-colors group',
        isOnboarding ? 'cursor-default' : 'cursor-pointer tint-interactive',
      ].join(' ')}
    >
      {identity}
      {actions}
    </div>
  );
};
