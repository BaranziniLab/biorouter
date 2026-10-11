import { useEffect, useState, useMemo, useCallback, type CSSProperties } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { useConfig } from './ConfigContext';
import { BioRouterWordmark } from './icons/BioRouterWordmark';
import { toastService } from '../toasts';
import ProviderCatalog from './settings/providers/ProviderCatalog';
import { persistDetectedProviderSetup } from './onboarding/CommercialSetupCard';
import type { DetectedProviderSetup } from './onboarding/CommercialSetupCard';
import { ProviderDetails } from '../api';
import { createNavigationHandler } from '../utils/navigationUtils';
import { isBrowserSurface } from '../utils/surface';
import { HostManagedModelPanel } from './privacy/HostManagedModelPanel';
import { getTitlebarControlInset } from './Layout/TitlebarControls';
import { InfoTip } from './ui/info-tip';
import { Button } from './ui/button';
import { ScrollArea } from './ui/scroll-area';
import { firstRunCopy } from './onboarding/copy';
import { providerPageCopy } from './settings/providers/copy';
import { KEY_STORAGE_NOTICE } from './settings/providers/modal/subcomponents/SecureStorageNotice';
import './onboarding/onboarding.css';

/**
 * The config key that records "let me in without a provider".
 *
 * ⚠ **Not a secret, and deliberately a config key rather than `localStorage`.**
 * It is a property of this install, not of this browser profile: a user who
 * skipped setup on Monday must not meet the first-run wall again after a
 * renderer reset. It is cleared the moment a provider is chosen, so a machine
 * that is set up never carries a stale "skipped" flag that would suppress the
 * wall if the provider were later removed.
 */
export const ONBOARDING_SKIPPED_KEY = 'BIOROUTER_ONBOARDING_SKIPPED';

interface ProviderGuardProps {
  didSelectProvider: boolean;
  children: React.ReactNode;
}

export default function ProviderGuard({ didSelectProvider, children }: ProviderGuardProps) {
  const { read, upsert, getProviders } = useConfig();
  const navigate = useNavigate();
  const location = useLocation();
  const [isChecking, setIsChecking] = useState(true);
  const [hasProvider, setHasProvider] = useState(false);
  const [showFirstTimeSetup, setShowFirstTimeSetup] = useState(false);
  const [userInActiveSetup, setUserInActiveSetup] = useState(false);
  // `null` until the daemon has answered: the catalog draws its skeleton, never an empty tab.
  const [providers, setProviders] = useState<ProviderDetails[] | null>(null);
  // The band's tab slot, which the catalog draws Local, Institutional and Public into
  // (`ProviderCatalog.tabsHost`). State, not a ref: the catalog re-renders once it exists.
  const [tabsHost, setTabsHost] = useState<HTMLElement | null>(null);

  const setView = useMemo(() => createNavigationHandler(navigate), [navigate]);

  const handleCommercialSuccess = async (setup: DetectedProviderSetup) => {
    await persistDetectedProviderSetup(upsert, setup);
  };

  const handleModelSelected = () => {
    setUserInActiveSetup(false);
    setShowFirstTimeSetup(false);
    setHasProvider(true);
    navigate('/', { replace: true });
  };

  /** Llama Server and Ollama configure themselves *and* pick their own model. */
  const handleLocalComplete = () => {
    setUserInActiveSetup(false);
    setShowFirstTimeSetup(false);
    setHasProvider(true);
    navigate('/', { replace: true });
  };

  /**
   * "Explore Biorouter first" — the app renders with no provider bound.
   *
   * ⚠ **The wall exists to stop a confusing failure, not to gate the product.**
   * Everything that does not need a model — Home, sessions, the Knowledge view,
   * settings, extensions — works perfectly well unconfigured, and a first-run
   * screen with no way past it turned "I want to look at this before pasting a
   * key" into "I cannot open the application". What the composer then owes the
   * user is the *reason* they cannot send, at the moment they try, which is what
   * `ChatInput`'s no-model hint and the model chip's "Choose a model" are for.
   */
  const handleSkip = useCallback(async () => {
    try {
      await upsert(ONBOARDING_SKIPPED_KEY, true, false);
      setShowFirstTimeSetup(false);
      navigate('/', { replace: true });
    } catch (error) {
      console.error('Failed to record the skipped onboarding:', error);
      toastService.error({
        title: 'Could not continue without a provider',
        msg: 'Biorouter could not save that choice, so the setup screen is still showing.',
        traceback: error instanceof Error ? error.stack || '' : '',
      });
    }
  }, [navigate, upsert]);

  useEffect(() => {
    const checkProvider = async () => {
      try {
        const provider = ((await read('BIOROUTER_PROVIDER', false)) as string) || '';
        const hasConfiguredProvider = provider.trim() !== '';
        const wasSkipped = (await read(ONBOARDING_SKIPPED_KEY, false)) === true;

        // Choosing a provider retires the skip: leaving it set would mean a
        // machine that later loses its provider silently skips the wall it now
        // needs.
        if (hasConfiguredProvider && wasSkipped) {
          try {
            await upsert(ONBOARDING_SKIPPED_KEY, false, false);
          } catch (error) {
            console.error('Failed to clear the skipped-onboarding flag:', error);
          }
        }

        if (userInActiveSetup) {
          setHasProvider(false);
          setShowFirstTimeSetup(true);
        } else if (hasConfiguredProvider || didSelectProvider) {
          setHasProvider(true);
          setShowFirstTimeSetup(false);
        } else {
          setHasProvider(false);
          setShowFirstTimeSetup(!wasSkipped);
        }
      } catch (error) {
        console.error('Error checking provider:', error);
        toastService.error({
          title: 'Configuration error',
          msg: 'Failed to check provider configuration.',
          traceback: error instanceof Error ? error.stack || '' : '',
        });
        setHasProvider(false);
        setShowFirstTimeSetup(true);
      } finally {
        setIsChecking(false);
      }
    };

    checkProvider();
  }, [read, upsert, didSelectProvider, userInActiveSetup]);

  /**
   * The catalog needs the daemon's provider list. Fetched only once the wall is
   * actually going to render — an already-configured install must not pay for a
   * provider listing (which constructs every configured provider) on every
   * launch just to render its children.
   */
  const needsCatalog = !isChecking && !hasProvider && showFirstTimeSetup && !isBrowserSurface();
  useEffect(() => {
    if (!needsCatalog) return;
    let cancelled = false;
    (async () => {
      try {
        const result = await getProviders(false);
        if (!cancelled) setProviders(result ?? []);
      } catch (error) {
        console.error('Failed to load providers for onboarding:', error);
        if (!cancelled) setProviders([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [needsCatalog, getProviders]);

  /**
   * Re-read the list after a setup changed it (a key saved, an agent's command written), so a
   * row's "Configured" follows what was just done. The first read is the effect above.
   */
  const refreshProviders = useCallback(() => {
    getProviders(true)
      .then((result) => {
        if (result) setProviders(result);
      })
      .catch((error: unknown) => console.error('Failed to refresh providers:', error));
  }, [getProviders]);

  const resumingSession =
    location.pathname === '/pair' &&
    Boolean(new URLSearchParams(location.search).get('resumeSessionId')?.trim());
  if (location.pathname === '/crew' || resumingSession) return <>{children}</>;

  if (isChecking) {
    /**
     * The loader draws nothing, on the page ground everything after it stands on (vision FR-R1-7,
     * R-29: the wordmark is the logo). It is up for a frame or two while the provider is read, so
     * it is a hand-off, not a screen: it used to repeat the boot splash's 84px BR monogram on the
     * recessed grey, then the first-run page (or the app shell) replaced both the mark and the
     * ground, a second logo and a second ground in one launch. On the page ground with nothing on
     * it, the splash fades straight to the ground the page then draws on, and the first logo a
     * person reads is the wordmark: in the first-run band, or at the head of the sidebar.
     */
    return (
      <div
        className="h-screen w-full bg-background-canvas"
        aria-busy="true"
        data-testid="provider-guard-loading"
      />
    );
  }

  if (!hasProvider && showFirstTimeSetup) {
    /**
     * SD-1's dead end, closed. Every path in the catalog below writes
     * `BIOROUTER_PROVIDER`, and on a browser-served surface all of those writes
     * are refused with a 409 whose body is addressed to an AI agent. Offering
     * the picker and then refusing it is the exact failure SD-1 rules out, so
     * the browser gets the one instruction that actually works — go and run it
     * on the host — and the catalog is not rendered at all. The panel carries the
     * reason itself, so the band adds no sentence of its own above it (FR-S-13).
     */
    const hostManaged = isBrowserSurface();
    const title = hostManaged ? firstRunCopy.hostTitle : firstRunCopy.title;

    /**
     * The first-run page is the redesigned app's page (R-34, vision FR-R1-2): one 44px band, then
     * the reading column on the page ground, as Settings and the `#/welcome` page are. It used to
     * be a 235px hero (a 36px wordmark, a 24px title, a two-line paragraph and two arrow links)
     * that never scrolled, on the recessed grey ground the boot splash uses.
     *
     * The band is the shared page band (`.biorouter-page-header[data-band]`, `PageHeader`'s
     * markup), written out here only because it leads with the wordmark, which is the logo
     * (R-29) and which `PageHeader` has no slot for: the 14px title with its InfoTip, the
     * catalog's three tabs drawn into its slot (`ProviderCatalog.tabsHost`), and the two ways out
     * as ghost buttons at its trailing edge. The page is outside the app shell like the provider
     * page, so it carries `.br-provider-page` for the same two rules (`providers.css`): the band's
     * lead clears the macOS traffic lights, and the 32px titlebar strip stops taking the band's
     * clicks. The loader above stands on this same page ground, empty (FR-R1-7).
     */
    const bandLead = {
      '--br-provider-band-lead': `${getTitlebarControlInset(
        (window?.electron?.platform || 'darwin') === 'darwin'
      )}px`,
    } as CSSProperties;

    return (
      <div
        className="br-provider-page br-first-run flex h-screen w-full flex-col bg-background-canvas text-text-default"
        style={bandLead}
      >
        <header
          className="biorouter-page-header br-ground-recessed"
          data-band=""
          data-testid="first-run-band"
        >
          <div className="biorouter-page-header-bar">
            {/* At rest on every mount: the page appears because of a launch or a return from
                Crew, and principle 9 has such a surface appear still (FR-S-15). */}
            <BioRouterWordmark className="br-first-run-wordmark" data-testid="first-run-wordmark" />
            <div className="biorouter-page-header-title">
              <h1 className="min-w-0 truncate text-label text-text-default">{title}</h1>
              {hostManaged ? null : (
                <InfoTip label={title} side="bottom" align="start">
                  {`${providerPageCopy.onboardingHelp} ${KEY_STORAGE_NOTICE}`}
                </InfoTip>
              )}
            </div>
            {hostManaged ? null : (
              <>
                <div ref={setTabsHost} className="br-provider-band-tabs-host" />
                {/* The skip is offered for the desktop application only. On a browser surface
                    there is nothing to explore *and* nothing the user can do about it from here:
                    the host owns the choice, and a "continue without a provider" that led to a
                    chat which can never be configured from this tab would be a second dead end
                    dressed as an escape. Said once, in the band: the foot copy is gone. */}
                <div className="biorouter-page-header-actions">
                  <Button
                    variant="ghost"
                    size="sm"
                    data-testid="onboarding-skip-header"
                    onClick={() => void handleSkip()}
                  >
                    {firstRunCopy.skip}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    data-testid="onboarding-open-crew"
                    onClick={() => navigate('/crew')}
                  >
                    {firstRunCopy.openCrew}
                  </Button>
                </div>
              </>
            )}
          </div>
        </header>

        <ScrollArea className="w-full flex-1">
          <div className="mx-auto w-full max-w-measure-chat px-6 py-6">
            {hostManaged ? (
              <HostManagedModelPanel />
            ) : (
              <ProviderCatalog
                tabsHost={tabsHost}
                providers={providers ?? []}
                loading={providers === null}
                holdUntilSettled
                refreshProviders={refreshProviders}
                mode="onboarding"
                setView={setView}
                onModelSelected={handleModelSelected}
                onLocalComplete={handleLocalComplete}
                onStartTesting={() => setUserInActiveSetup(true)}
                onCommercialSuccess={handleCommercialSuccess}
                configuredProvider={null}
              />
            )}
          </div>
        </ScrollArea>
      </div>
    );
  }

  return <>{children}</>;
}
