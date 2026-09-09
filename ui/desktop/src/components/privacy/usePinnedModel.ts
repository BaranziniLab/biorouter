import { useEffect, useState } from 'react';
import { useConfig } from '../ConfigContext';
import { useModelAndProvider } from '../ModelAndProviderContext';
import type { PinnedModelView } from '../../hooks/chatStreamStore';
import { bindingLabel, pinContradictsSelection, pinnedModelNotice } from './pinnedModel';

export interface PinnedModelPresentation {
  /** The binding the chat is pinned to, or `undefined` when nothing is. */
  pinned?: PinnedModelView;
  /**
   * The one line to show the user, or `null` when there is nothing to say —
   * either nothing is pinned, or the pin names exactly what is already on
   * screen, or the selection has not resolved yet.
   */
  notice: string | null;
}

/**
 * Resolve the pin (issue #56 Gate B) into the sentence the composer shows, or
 * into `null`.
 *
 * ⚠ The provider's **display name** is resolved through `getProviders`, which
 * `ConfigContext` caches, rather than being derived from the id. `versa_azure`
 * is not what that provider is called anywhere else in the app, and a note that
 * named it that way would read as an internal leak in the one place the user is
 * being told something they did not already know.
 *
 * ⚠ A provider the catalog cannot name falls back to the model alone. The
 * sentence stays true and complete; inventing a display name from the id would
 * not.
 */
export function usePinnedModel(pinned: PinnedModelView | undefined): PinnedModelPresentation {
  const { getProviders } = useConfig();
  const { currentModel, currentProvider } = useModelAndProvider();
  const [displayNames, setDisplayNames] = useState<Record<string, string>>({});

  const contradicts = pinContradictsSelection(pinned, {
    provider: currentProvider,
    model: currentModel,
  });
  const pinnedProvider = pinned?.provider;
  const selectedProvider = currentProvider ?? undefined;

  useEffect(() => {
    // ⚠ Nothing is fetched unless there is a sentence to build. This hook runs
    // in every chat, on every render of the composer, and the overwhelmingly
    // common answer is "no note" — a catalog read there would be pure cost, and
    // a state update behind it would churn the composer for no visible change.
    if (!contradicts) return;
    // Both halves of the sentence name a provider, and both are ids until this
    // lands. Resolved in ONE pass over ONE fetch so the two names can never
    // come from different reads of the catalog.
    const wanted = [pinnedProvider, selectedProvider].filter((id): id is string => !!id);
    if (wanted.length === 0) return;
    let cancelled = false;
    (async () => {
      try {
        const rows = await getProviders(false);
        if (cancelled) return;
        const resolved: Record<string, string> = {};
        for (const id of wanted) {
          const name = rows.find((row) => row.name === id)?.metadata?.display_name;
          if (name) resolved[id] = name;
        }
        setDisplayNames((prev) => {
          const unchanged = wanted.every((id) => prev[id] === resolved[id]);
          return unchanged ? prev : { ...prev, ...resolved };
        });
      } catch {
        // A catalog we cannot read costs the display names and nothing else:
        // the ids below are still true, and staying silent about the pin would
        // be the one outcome that reintroduces the defect.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [contradicts, getProviders, pinnedProvider, selectedProvider]);

  if (!contradicts) return { pinned, notice: null };

  return {
    pinned,
    notice: pinnedModelNotice(
      bindingLabel(displayNames[pinned!.provider], pinned!.model),
      bindingLabel(selectedProvider ? displayNames[selectedProvider] : undefined, currentModel!)
    ),
  };
}
