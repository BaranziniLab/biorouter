import { useEffect, useState } from 'react';
import type { ProviderDetails } from '../../../api';
import { useConfig } from '../../ConfigContext';

/** A provider and one of its models, as the run request names them. */
export interface ModelChoice {
  provider: string;
  model: string;
}

export interface ConfiguredModels {
  /** Configured providers, in the daemon's order; `null` until the list has loaded. */
  providers: ProviderDetails[] | null;
  /** `BIOROUTER_PROVIDER` / `BIOROUTER_MODEL`, only when both resolve to a configured provider. */
  defaults: ModelChoice | null;
  /** The list could not be loaded; the message is the daemon's. */
  failure: string | null;
}

/** A provider's display name, falling back to its key for a row with no metadata. */
export function providerLabel(provider: ProviderDetails | undefined, fallback = ''): string {
  const name = provider?.metadata?.display_name;
  return typeof name === 'string' && name.trim() ? name : (provider?.name ?? fallback);
}

/**
 * The configured providers and the app's default model, read when Ask my agent opens.
 *
 * Crew bypasses provider onboarding, so a person can arrive here with nothing configured: then
 * `providers` is an empty list, which the pane turns into "No models are set up." A failed read is
 * reported separately and never claims that nothing is set up.
 */
export function useConfiguredModels(): ConfiguredModels {
  const { getProviders, read } = useConfig();
  const [state, setState] = useState<ConfiguredModels>({
    providers: null,
    defaults: null,
    failure: null,
  });

  useEffect(() => {
    let active = true;
    void Promise.all([
      getProviders(false),
      read('BIOROUTER_PROVIDER', false),
      read('BIOROUTER_MODEL', false),
    ])
      .then(([items, provider, model]) => {
        if (!active) return;
        const configured = (Array.isArray(items) ? items : []).filter(
          (item): item is ProviderDetails =>
            Boolean(item) && typeof item.name === 'string' && item.is_configured === true
        );
        const defaults =
          typeof provider === 'string' &&
          typeof model === 'string' &&
          model.trim() !== '' &&
          configured.some((item) => item.name === provider)
            ? { provider, model: model.trim() }
            : null;
        setState({ providers: configured, defaults, failure: null });
      })
      .catch((failure: unknown) => {
        if (!active) return;
        setState({
          providers: null,
          defaults: null,
          failure: failure instanceof Error && failure.message ? failure.message : String(failure),
        });
      });
    return () => {
      active = false;
    };
  }, [getProviders, read]);

  return state;
}
