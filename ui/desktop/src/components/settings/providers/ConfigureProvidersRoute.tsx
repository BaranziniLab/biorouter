import { useLocation, useNavigate } from 'react-router-dom';
import ProviderSettings from './ProviderSettingsPage';
import type { ViewOptions } from '../../../utils/navigationUtils';

/**
 * `#/configure-providers`: the provider catalog outside onboarding.
 *
 * W2-PRV-5. Opened from a chat's model picker ("Use other provider"), the
 * catalog is a detour: it goes back where it came from (`returnTo`), and the
 * model chosen after a setup here is chosen for that chat (`resumeSessionId`,
 * with its tier for the picker's pre-flight). It used to land on Settings >
 * Models with no way back, and set the model every new chat starts on. Opened
 * from anywhere else it behaves as before, and Back goes to Settings > Models.
 */
export default function ConfigureProvidersRoute() {
  const navigate = useNavigate();
  const location = useLocation();
  const from = (location.state ?? {}) as ViewOptions;
  const returnTo = from.returnTo;
  const goBack = () =>
    returnTo ? navigate(returnTo) : navigate('/settings', { state: { section: 'models' } });

  return (
    <div className="w-screen h-screen bg-background-default">
      <ProviderSettings
        onClose={goBack}
        isOnboarding={false}
        chatSessionId={from.resumeSessionId ?? null}
        chatPrivacyTier={from.privacyTier}
        onProviderLaunched={returnTo ? () => navigate(returnTo) : undefined}
      />
    </div>
  );
}
