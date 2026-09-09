import { useEffect, useState, useCallback, useRef, useMemo } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { ScrollArea } from '../../ui/scroll-area';
import BackButton from '../../ui/BackButton';
import ProviderCatalog from './ProviderCatalog';
import { useConfig } from '../../ConfigContext';
import { ProviderDetails } from '../../../api';
import { createNavigationHandler } from '../../../utils/navigationUtils';
import { persistDetectedProviderSetup } from '../../onboarding/CommercialSetupCard';
import type { DetectedProviderSetup } from '../../onboarding/CommercialSetupCard';

interface ProviderSettingsProps {
  onClose: () => void;
  isOnboarding: boolean;
  onProviderLaunched?: (model?: string) => void;
}

export default function ProviderSettings({
  onClose,
  isOnboarding,
  onProviderLaunched,
}: ProviderSettingsProps) {
  const { getProviders, read, upsert } = useConfig();
  const navigate = useNavigate();
  // HashRouter, so the hint lives in the fragment's own query string:
  // `#/configure-providers?tab=public`.
  const { search } = useLocation();
  const [loading, setLoading] = useState(true);
  const [providers, setProviders] = useState<ProviderDetails[]>([]);
  const [configuredProvider, setConfiguredProvider] = useState<string | null>(null);
  const initialLoadDone = useRef(false);

  const setView = useMemo(() => createNavigationHandler(navigate), [navigate]);
  const tabHint = useMemo(() => new URLSearchParams(search).get('tab'), [search]);

  // Create a function to load providers that can be called multiple times
  const loadProviders = useCallback(async () => {
    setLoading(true);
    try {
      // Only force refresh when explicitly requested, not on initial load
      const result = await getProviders(!initialLoadDone.current);
      if (result) {
        setProviders(result);
        initialLoadDone.current = true;
      }
    } catch (error) {
      console.error('Failed to load providers:', error);
    } finally {
      setLoading(false);
    }
  }, [getProviders]);

  // Load providers only once when component mounts
  useEffect(() => {
    loadProviders();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // Intentionally not including loadProviders in deps to prevent reloading

  /**
   * Which provider is bound right now — read once, purely to pick the opening
   * tab. A failure is not worth a toast: the cost is landing on the Local tab
   * instead of the user's own, which the tabs themselves fix in one click.
   */
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const value = ((await read('BIOROUTER_PROVIDER', false)) as string) || '';
        if (!cancelled) setConfiguredProvider(value.trim() || null);
      } catch {
        if (!cancelled) setConfiguredProvider(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [read]);

  // This function will be passed to the catalog for manual refreshes after config changes
  const refreshProviders = useCallback(() => {
    if (initialLoadDone.current) {
      getProviders(true).then((result) => {
        if (result) setProviders(result);
      });
    }
  }, [getProviders]);

  /**
   * The `welcome` route reaches the onboarding catalog without a `ProviderGuard`
   * above it, so the key-detection write has to happen here too — through the
   * same helper the guard uses, never a second copy of the sequence.
   */
  const handleCommercialSuccess = useCallback(
    async (setup: DetectedProviderSetup) => {
      await persistDetectedProviderSetup(upsert, setup);
      refreshProviders();
    },
    [refreshProviders, upsert]
  );

  return (
    <div className="h-screen w-full flex flex-col bg-background-muted text-text-default">
      <ScrollArea className="flex-1 w-full">
        {/* Flat page header, on the chat measure — the same reading column the
            rest of the app uses, rather than this page's own width. */}
        <div className="w-full max-w-measure-chat mx-auto px-6 pt-10 pb-6 border-b border-border-subtle">
          <div className="flex items-center mb-4 no-drag">
            <BackButton onClick={onClose} />
          </div>
          <h1 className="text-title mb-1" data-testid="provider-selection-heading">
            {isOnboarding ? 'Choose a provider' : 'Provider configuration'}
          </h1>
          <p className="text-sm text-text-muted">
            {isOnboarding
              ? 'Pick where your models run. API keys are encrypted and stored locally, and you can switch providers any time in settings.'
              : 'Configure your AI model providers. API keys are encrypted and stored locally.'}
          </p>
        </div>

        <div className="w-full max-w-measure-chat mx-auto px-6 py-6">
          {loading ? (
            <div className="text-sm text-text-muted">Loading providers…</div>
          ) : (
            <ProviderCatalog
              providers={providers}
              mode={isOnboarding ? 'onboarding' : 'settings'}
              refreshProviders={refreshProviders}
              setView={setView}
              onModelSelected={onProviderLaunched}
              onLocalComplete={() => onProviderLaunched?.()}
              onCommercialSuccess={handleCommercialSuccess}
              configuredProvider={configuredProvider}
              initialTab={tabHint}
            />
          )}
        </div>
      </ScrollArea>
    </div>
  );
}
