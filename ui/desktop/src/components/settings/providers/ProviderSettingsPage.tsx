import { useEffect, useState, useCallback, useRef, useMemo, type CSSProperties } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { ScrollArea } from '../../ui/scroll-area';
import { PageHeader } from '../../Layout/PageHeader';
import { getTitlebarControlInset } from '../../Layout/TitlebarControls';
import ProviderCatalog from './ProviderCatalog';
import { useConfig } from '../../ConfigContext';
import { ProviderDetails, type SessionClassification } from '../../../api';
import { createNavigationHandler } from '../../../utils/navigationUtils';
import { persistDetectedProviderSetup } from '../../onboarding/CommercialSetupCard';
import type { DetectedProviderSetup } from '../../onboarding/CommercialSetupCard';
import { KEY_STORAGE_NOTICE } from './modal/subcomponents/SecureStorageNotice';
import { providerPageCopy } from './copy';
import './providers.css';

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
  // `undefined` until read: the catalog holds its opening tab until it knows (`holdUntilSettled`).
  const [configuredProvider, setConfiguredProvider] = useState<string | null | undefined>(
    undefined
  );
  const initialLoadDone = useRef(false);
  // The band's tab slot, which the catalog draws its tabs into (`ProviderCatalog.tabsHost`). State
  // rather than a ref: the catalog must re-render when the element exists.
  const [tabsHost, setTabsHost] = useState<HTMLElement | null>(null);

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
  // The band's InfoTip: where a key is kept, which the key field says again where one is typed.
  // First run adds the one thing worth knowing before choosing: the choice is not final.
  const help = isOnboarding
    ? `${providerPageCopy.onboardingHelp} ${KEY_STORAGE_NOTICE}`
    : KEY_STORAGE_NOTICE;

  // Where the band's first control starts, clear of the window controls. The page is outside the
  // app shell, so no sidebar's titlebar band holds the macOS traffic lights; the band does. The
  // number is the one the chat header's collapsed sidebar reserves for the same lights
  // (`getTitlebarControlInset`: 100 on macOS, 16 elsewhere, and a browser reports `linux`).
  const isMacOS = (window?.electron?.platform || 'darwin') === 'darwin';
  const bandLead = useMemo(
    () => ({ '--br-provider-band-lead': `${getTitlebarControlInset(isMacOS)}px` }) as CSSProperties,
    [isMacOS]
  );

  return (
    // The same ground as every other view (`MainPanelLayout`'s canvas), not a muted one: this
    // page used to be a shade darker than the Settings it was opened from.
    <div
      className="br-provider-page flex h-screen w-full flex-col bg-background-canvas text-text-default"
      style={bandLead}
    >
      {/* One 44px band at the top edge, as every other page has (principle 1): Back as a 32px icon
          action, the 14px title with its help, and the three tabs on its line. The page is outside
          the app shell (`routeOwnsTopBand`), so the band is its own window-drag region: it starts
          at y=0, its leading edge is the titlebar reserve (`--br-provider-band-lead`), and
          `providers.css` stops the 32px strip `App.tsx` paints from taking its clicks. There is no
          spacer under that strip any more; it stood in for the band's missing 32px and made the
          page 76px of chrome (vision round 2, V4-14). */}
      <PageHeader
        title={heading}
        info={help}
        onBack={onClose}
        tabs={<div ref={setTabsHost} className="br-provider-band-tabs-host" />}
      />
      {/* `br-page-headroom`: the clear space under the band at the top, the one Settings and the
          first-run page keep too (`--page-headroom`). */}
      <ScrollArea className="flex-1 w-full">
        <div className="br-page-headroom mx-auto w-full max-w-measure-chat px-6 pb-6">
          {/* Mounted at once, so the coding-agent probe runs while the list loads; the catalog
              draws the skeleton until it knows which tab opens, then opens there at rest. */}
          <ProviderCatalog
            tabsHost={tabsHost}
            providers={providers}
            loading={loading}
            holdUntilSettled
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
        </div>
      </ScrollArea>
    </div>
  );
}
