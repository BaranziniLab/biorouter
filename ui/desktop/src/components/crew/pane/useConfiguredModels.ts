import { useCallback, useEffect, useState } from 'react';
import type { ProviderDetails } from '../../../api';
import { subscribeAppModelSelectionChanges } from '../../../utils/sessionBindingSync';
import { useConfig } from '../../ConfigContext';
import {
  CREW_TASK_MODEL_KEY,
  resolveTaskModel,
  type TaskModelChoice,
  type TaskModelSource,
} from '../state/crewTaskModel';

/** A provider and one of its models, as the run request names them. */
export type ModelChoice = TaskModelChoice;

export interface ConfiguredModels {
  /** Configured providers, in the daemon's order; `null` until the list has loaded. */
  providers: ProviderDetails[] | null;
  /** `BIOROUTER_PROVIDER` / `BIOROUTER_MODEL`, only when both resolve to a configured provider. */
  defaults: ModelChoice | null;
  /**
   * The model a Crew task starts on (OW-11): `CREW_TASK_MODEL`, else the app's default, each only
   * when it names a configured provider that can run a Crew task (never Claude Code or Codex).
   */
  taskDefault: ModelChoice | null;
  /** Where `taskDefault` came from, or `null` with it. */
  taskDefaultSource: TaskModelSource | null;
  /** The list could not be loaded; the message is the daemon's. */
  failure: string | null;
  /**
   * Make `choice` the model for Crew tasks (`CREW_TASK_MODEL`). Absent where the configuration
   * cannot be written (outside a `ConfigProvider`). Rejects when the daemon refused the write.
   */
  saveTaskDefault?(choice: ModelChoice): Promise<void>;
}

/**
 * The app's configuration, or `null` outside a `ConfigProvider` when `optional` — for a surface
 * that only adds to what it says with it (the Chat access pane's model line), never one that needs
 * it. `useConfig` reads the context before it throws, so the hook order is the same either way.
 */
function useOptionalConfig(optional: boolean): ReturnType<typeof useConfig> | null {
  try {
    return useConfig();
  } catch (failure) {
    if (optional) return null;
    throw failure;
  }
}

/** A provider's display name, falling back to its key for a row with no metadata. */
export function providerLabel(provider: ProviderDetails | undefined, fallback = ''): string {
  const name = provider?.metadata?.display_name;
  return typeof name === 'string' && name.trim() ? name : (provider?.name ?? fallback);
}

const EMPTY: ConfiguredModels = {
  providers: null,
  defaults: null,
  taskDefault: null,
  taskDefaultSource: null,
  failure: null,
};

/**
 * The configured providers, the app's default model and the model for Crew tasks, read when Ask
 * my agent opens (or, for the composer, the first time `@agent` needs them: `enabled`).
 *
 * Crew bypasses provider onboarding, so a person can arrive here with nothing configured: then
 * `providers` is an empty list, which the pane turns into "No models are set up." A failed read is
 * reported separately and never claims that nothing is set up.
 *
 * Read again whenever a window announces a write of the app's model selection or of
 * `CREW_TASK_MODEL` (`ConfigContext` announces both), so a change made in Settings reaches a pane
 * or a composer that is already open.
 */
export function useConfiguredModels(
  options: { optional?: boolean; enabled?: boolean } = {}
): ConfiguredModels {
  const config = useOptionalConfig(options.optional === true);
  const getProviders = config?.getProviders;
  const read = config?.read;
  const upsert = config?.upsert;
  const enabled = options.enabled !== false;
  const [state, setState] = useState<ConfiguredModels>(EMPTY);
  // Once asked for, the answer is kept and re-read on every announcement, whether or not the
  // caller still wants it: a composer that types `@agent` again finds the model it showed.
  const [wanted, setWanted] = useState(enabled);
  useEffect(() => {
    if (enabled) setWanted(true);
  }, [enabled]);
  const [version, setVersion] = useState(0);
  useEffect(() => {
    if (!wanted) return;
    return subscribeAppModelSelectionChanges(() => setVersion((current) => current + 1));
  }, [wanted]);

  useEffect(() => {
    if (!wanted || !getProviders || !read) return;
    let active = true;
    void Promise.all([
      getProviders(false),
      read('BIOROUTER_PROVIDER', false),
      read('BIOROUTER_MODEL', false),
      read(CREW_TASK_MODEL_KEY, false),
    ])
      .then(([items, provider, model, crew]) => {
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
        const task = resolveTaskModel({
          crew,
          appProvider: provider,
          appModel: model,
          configured,
        });
        setState({
          providers: configured,
          defaults,
          taskDefault: task?.choice ?? null,
          taskDefaultSource: task?.source ?? null,
          failure: null,
        });
      })
      .catch((failure: unknown) => {
        if (!active) return;
        setState({
          ...EMPTY,
          failure: failure instanceof Error && failure.message ? failure.message : String(failure),
        });
      });
    return () => {
      active = false;
    };
  }, [wanted, getProviders, read, version]);

  const saveTaskDefault = useCallback(
    async (choice: ModelChoice) => {
      if (!upsert) return;
      // `upsert` announces the write to every other surface once it resolved; this one reads
      // again for itself, so what it says next is what the daemon now holds.
      await upsert(CREW_TASK_MODEL_KEY, { provider: choice.provider, model: choice.model }, false);
      setVersion((current) => current + 1);
    },
    [upsert]
  );

  return upsert ? { ...state, saveTaskDefault } : state;
}
