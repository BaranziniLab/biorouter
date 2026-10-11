import { ConfigureSettingsButton, RocketButton } from './CardButtons';
import { ProviderDetails } from '../../../../../api';
import { catalogCopy } from '../../copy';

// can define other optional callbacks as needed
interface CardButtonsProps {
  provider: ProviderDetails;
  isOnboardingPage: boolean;
  onConfigure: (provider: ProviderDetails) => void;
  onLaunch: (provider: ProviderDetails) => void;
  /** The id of the row's full description, so the button is described by it (FR-R1-9). */
  describedBy?: string;
}

/**
 * A provider row's one action. The visible word is short ("Configure", "Launch"); the accessible
 * name carries the row, so a list of twenty rows is not twenty buttons called "Configure". A
 * tooltip that only repeated that name was dropped: the row already says whose it is.
 */
export default function DefaultCardButtons({
  provider,
  isOnboardingPage,
  onLaunch,
  onConfigure,
  describedBy,
}: CardButtonsProps) {
  const name = provider.metadata.display_name || provider.name;
  // A configured provider on the onboarding page is launched; every other row is configured
  // (set up, or edited when it already is).
  if (provider.is_configured && isOnboardingPage) {
    return (
      <RocketButton
        aria-label={catalogCopy.launchRow(name)}
        aria-describedby={describedBy}
        onClick={(e) => {
          e.stopPropagation();
          onLaunch(provider);
        }}
      />
    );
  }
  return (
    <ConfigureSettingsButton
      aria-label={catalogCopy.configureRow(name)}
      aria-describedby={describedBy}
      onClick={(e) => {
        e.stopPropagation();
        onConfigure(provider);
      }}
    />
  );
}
