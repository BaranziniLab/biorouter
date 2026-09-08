import { useEffect, useState, useMemo, useCallback } from 'react';
import { useNavigate } from 'react-router-dom';
import { useConfig } from './ConfigContext';
import { BioRouterMark } from './icons/BioRouterMark';
import { BioRouterWordmark } from './icons/BioRouterWordmark';
import { toastService } from '../toasts';
import ProviderCatalog from './settings/providers/ProviderCatalog';
import { persistDetectedProviderSetup } from './onboarding/CommercialSetupCard';
import type { DetectedProviderSetup } from './onboarding/CommercialSetupCard';
import { ProviderDetails } from '../api';
import { createNavigationHandler } from '../utils/navigationUtils';
import { isBrowserSurface } from '../utils/surface';
import { HostManagedModelPanel } from './privacy/HostManagedModelPanel';
import { HOST_SERVE_COMMAND } from './privacy/hostManagedModelCopy';

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
  const [isChecking, setIsChecking] = useState(true);
  const [hasProvider, setHasProvider] = useState(false);
  const [showFirstTimeSetup, setShowFirstTimeSetup] = useState(false);
  const [userInActiveSetup, setUserInActiveSetup] = useState(false);
  const [providers, setProviders] = useState<ProviderDetails[]>([]);

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
        if (!cancelled && result) setProviders(result);
      } catch (error) {
        console.error('Failed to load providers for onboarding:', error);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [needsCatalog, getProviders]);

  if (isChecking) {
    return (
      <div className="h-screen w-full bg-background-muted flex items-center justify-center">
        {/* 84px, not Tailwind's h-20 (80px), to match the pre-React boot
            splash's `.br-mark` exactly. This loader is what the splash hands
            off to, so a 4px difference read as the logo resizing mid-boot.
            The splash centres the same mark on the same point — see the
            `.br-sweep` note in index.html. */}
        <BioRouterMark className="h-[84px] w-[84px]" />
      </div>
    );
  }

  if (!hasProvider && showFirstTimeSetup) {
    /**
     * SD-1's dead end, closed. Every path in the catalog below writes
     * `BIOROUTER_PROVIDER`, and on a browser-served surface all of those writes
     * are refused with a 409 whose body is addressed to an AI agent. Offering
     * the picker and then refusing it is the exact failure SD-1 rules out, so
     * the browser gets the one instruction that actually works — go and run it
     * on the host — and the catalog is not rendered at all.
     */
    const hostManaged = isBrowserSurface();
    /**
     * The skip is offered for the desktop application only. On a browser surface
     * there is nothing to explore *and* nothing the user can do about it from
     * here: the host owns the choice, and a "continue without a provider" that
     * led to a chat which can never be configured from this tab would be a
     * second dead end dressed as an escape.
     */
    /**
     * ⚠ Rendered TWICE — under the header and again at the foot — so the way out
     * is visible from wherever the user stopped reading rather than only from
     * the top. Each instance carries its own test id: one shared id would make
     * every `getByTestId` in the suites throw "found multiple elements", which
     * reads as a bug in the test rather than as two deliberate copies.
     */
    const skipAction = (placement: 'header' | 'foot') =>
      hostManaged ? null : (
        <button
          type="button"
          onClick={() => void handleSkip()}
          data-testid={`onboarding-skip-${placement}`}
          className="text-sm text-text-muted transition-colors duration-150 hover:text-text-default"
        >
          Explore Biorouter first →
        </button>
      );

    return (
      <div className="flex h-screen w-full flex-col overflow-hidden bg-background-muted">
        {/* Flat page header, on the same reading column as the panels below. */}
        <div className="flex-shrink-0 border-b border-border-subtle px-5 pb-5 pt-8 sm:px-6 sm:pb-6 sm:pt-10">
          <div className="mx-auto max-w-measure-chat">
            <div className="mb-4 sm:mb-5 biorouter-icon-animation origin-bottom-left">
              <BioRouterWordmark className="h-9 w-auto" />
            </div>
            <h1 className="text-2xl font-semibold tracking-tight text-text-default">
              Welcome to Biorouter
            </h1>
            <p className="text-sm text-text-muted mt-1.5 leading-relaxed">
              {hostManaged
                ? `Biorouter is being served to this browser by ${HOST_SERVE_COMMAND}. One more step is needed on that machine before you can start a chat.`
                : 'An integrated research environment that connects local, institution-hosted, and commercial AI models in one interface, built for biomedical discovery.'}
            </p>
            {!hostManaged && <div className="mt-4">{skipAction('header')}</div>}
          </div>
        </div>

        {/* Scrollable body */}
        <div className="flex-1 min-h-0 overflow-y-auto bg-background-muted">
          <div className="mx-auto max-w-measure-chat px-5 py-6 sm:px-6">
            {hostManaged ? (
              <HostManagedModelPanel />
            ) : (
              <>
                <ProviderCatalog
                  providers={providers}
                  mode="onboarding"
                  setView={setView}
                  onModelSelected={handleModelSelected}
                  onLocalComplete={handleLocalComplete}
                  onStartTesting={() => setUserInActiveSetup(true)}
                  onCommercialSuccess={handleCommercialSuccess}
                  configuredProvider={null}
                />
                <div className="mt-8 border-t border-border-subtle pt-5">{skipAction('foot')}</div>
              </>
            )}
          </div>
        </div>
      </div>
    );
  }

  return <>{children}</>;
}
