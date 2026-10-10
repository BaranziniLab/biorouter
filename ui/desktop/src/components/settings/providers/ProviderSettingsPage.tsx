import { useEffect, useState, useCallback, useRef, useMemo } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { ScrollArea } from '../../ui/scroll-area';
import BackButton from '../../ui/BackButton';
import { InfoTip } from '../../ui/info-tip';
import { Skeleton } from '../../ui/skeleton';
import ProviderCatalog from './ProviderCatalog';
import { useConfig } from '../../ConfigContext';
import { ProviderDetails, type SessionClassification } from '../../../api';
import { createNavigationHandler } from '../../../utils/navigationUtils';
import { persistDetectedProviderSetup } from '../../onboarding/CommercialSetupCard';
import type { DetectedProviderSetup } from '../../onboarding/CommercialSetupCard';
import { KEY_STORAGE_NOTICE } from './modal/subcomponents/SecureStorageNotice';
import { providerPageCopy } from './copy';

interface ProviderSettingsProps {
  onClose: () => void;
  isOnboarding: boolean;
  onProviderLaunched?: (model?: string) => void;
  /** W2-PRV-5: the chat whose model picker opened this page; see `ProviderCatalog`. */
  chatSessionId?: string | null;
  chatPrivacyTier?: SessionClassification;
  /** T3-SH-2: the tab of the unsent chat whose model picker opened this page. */
  heldChatTabId?: string | null;
}

export default function ProviderSettings({
  onClose,
  isOnboarding,
  onProviderLaunched,
  chatSessionId,
  chatPrivacyTier,
  heldChatTabId,
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

  const heading = isOnboarding ? providerPageCopy.onboardingTitle : providerPageCopy.title;

  return (
    // The same ground as every other view (`MainPanelLayout`'s canvas), not a muted one: this
    // page used to be a shade darker than the Settings it was opened from.
    <div className="flex h-screen w-full flex-col bg-background-canvas text-text-default">
      <ScrollArea className="flex-1 w-full">
        {/* Flat page header, on the chat measure — the same reading column the
            rest of the app uses, rather than this page's own width. Where a key is kept is the
            heading's InfoTip: it is said once, and the key field says it again where a key is
            typed. First run keeps its one lead-in line, the only text on the screen. */}
        <div className="mx-auto w-full max-w-measure-chat border-b border-border-subtle px-6 pb-4 pt-10">
          <div className="no-drag mb-4 flex items-center">
            <BackButton onClick={onClose} />
          </div>
          <div className="flex items-center">
            <h1 className="text-title" data-testid="provider-selection-heading">
              {heading}
            </h1>
            <InfoTip label={heading} help={KEY_STORAGE_NOTICE} />
          </div>
          {isOnboarding && (
            <p className="mt-1 text-secondary text-text-muted">{providerPageCopy.onboardingLead}</p>
          )}
        </div>

        <div className="mx-auto w-full max-w-measure-chat px-6 py-6">
          {loading ? (
            <div
              role="status"
              aria-label={providerPageCopy.loading}
              className="flex flex-col gap-3"
            >
              <Skeleton className="h-5 w-48" />
              <Skeleton className="h-10 w-full" />
              <Skeleton className="h-10 w-full" />
              <Skeleton className="h-10 w-full" />
            </div>
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
              chatSessionId={chatSessionId}
              chatPrivacyTier={chatPrivacyTier}
              heldChatTabId={heldChatTabId}
            />
          )}
        </div>
      </ScrollArea>
    </div>
  );
}
