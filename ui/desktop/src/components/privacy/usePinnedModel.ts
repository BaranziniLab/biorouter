import { useEffect, useState } from 'react';
import { useConfig } from '../ConfigContext';
import { useModelAndProvider } from '../ModelAndProviderContext';
import { readResolvedProviderTier } from './useBoundProviderTier';
import type { PinnedModelView } from '../../hooks/chatStreamStore';
import type { ProviderTier, SessionClassification } from '../../api/types.gen';
import {
  bindingDiffersFromSelection,
  bindingLabel,
  pinnedModelNotice,
  selectionBarredByPrivacy,
} from './pinnedModel';

export interface PinnedModelPresentation {
  /**
   * The one line to show the user, or `null` when there is nothing to say —
   * nothing differs, the difference is not the barrier's doing, or the facts
   * have not resolved yet.
   */
  notice: string | null;
}

/**
 * Resolve a chat's binding into the sentence the composer shows, or into `null`.
 *
 * ⚠ The provider's **display name** is resolved through `getProviders`, which
 * `ConfigContext` caches, rather than being derived from the id. `versa_azure`
 * is not what that provider is called anywhere else in the app, and a note that
 * named it that way would read as an internal leak in the one place the user is
 * being told something they did not already know.
 *
 * ⚠ The SELECTED provider's tier comes off the same fetch, and off the ROW
 * rather than `row.metadata` — `metadata.tier` is the type-level claim, and
 * `resolved_tier` is the instance-resolved one (DR-26). Reading the type-level
 * field would mis-classify an `ollama` re-pointed off this machine, which is the
 * exact demotion the tier exists to catch.
 *
 * ⚠ A provider the catalog cannot name falls back to the model alone. The
 * sentence stays true and complete; inventing a display name from the id would
 * not. A catalog that cannot be read at all yields no tier, so no note — saying
 * *why* a choice is not in effect requires knowing that it is not.
 */
export function usePinnedModel(
  binding: PinnedModelView | undefined,
  chatTier: SessionClassification | undefined
): PinnedModelPresentation {
  const { getProviders } = useConfig();
  const { currentModel, currentProvider } = useModelAndProvider();
  const [displayNames, setDisplayNames] = useState<Record<string, string>>({});
  const [selectedTier, setSelectedTier] = useState<ProviderTier | undefined>(undefined);

  const differs = bindingDiffersFromSelection(binding, {
    provider: currentProvider,
    model: currentModel,
  });
  const bindingProvider = binding?.provider;
  const selectedProvider = currentProvider ?? undefined;

  useEffect(() => {
    // ⚠ Nothing is fetched unless a difference exists to explain. This hook runs
    // in every chat, on every render of the composer, and the overwhelmingly
    // common answer is "no note" — a catalog read there would be pure cost, and
    // a state update behind it would churn the composer for no visible change.
    if (!differs) return;
    // Both halves of the sentence name a provider, and the tier decides whether
    // there is a sentence at all. Resolved in ONE pass over ONE fetch so the
    // three facts cannot come from different reads of the catalog.
    const wanted = [bindingProvider, selectedProvider].filter((id): id is string => !!id);
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
        const selectedRow = selectedProvider
          ? rows.find((row) => row.name === selectedProvider)
          : undefined;
        setSelectedTier(selectedRow ? readResolvedProviderTier(selectedRow) : undefined);
      } catch {
        // A catalog we cannot read leaves the tier unresolved, and an
        // unresolved tier means no note: the sentence claims to know WHY the
        // user's choice is not in effect, and here we do not.
        if (!cancelled) setSelectedTier(undefined);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [differs, getProviders, bindingProvider, selectedProvider]);

  if (!differs || !selectionBarredByPrivacy(chatTier, selectedTier)) {
    return { notice: null };
  }

  return {
    notice: pinnedModelNotice(
      bindingLabel(displayNames[binding!.provider], binding!.model),
      bindingLabel(selectedProvider ? displayNames[selectedProvider] : undefined, currentModel!)
    ),
  };
}
