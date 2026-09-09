import { useEffect, useState } from 'react';
import { useConfig } from '../ConfigContext';
import { useModelAndProvider } from '../ModelAndProviderContext';
import { readResolvedProviderTier } from './useBoundProviderTier';
import type { PinnedModelView } from '../../hooks/chatStreamStore';
import type { ProviderTier, Session } from '../../api/types.gen';
import {
  bindingDiffersFromSelection,
  bindingLabel,
  chatBinding,
  pinnedModelNotice,
  selectionBarredByPrivacy,
} from './pinnedModel';

export interface PinnedModelPresentation {
  /**
   * What the composer's model chip and context gauge should state, or
   * `undefined` to keep stating the app-wide selection.
   */
  effectiveModel?: PinnedModelView;
  /**
   * The one line to show the user, or `null` when there is nothing to say —
   * nothing differs, or the difference is not the barrier's doing, or the facts
   * have not resolved yet.
   */
  notice: string | null;
}

/**
 * What this chat runs on, and whether that needs explaining.
 *
 * # One rule
 *
 * **The composer states the chat's own binding exactly when the app-wide
 * selection cannot run here.** Otherwise the selection stands.
 *
 * ⚠ The second half is not a hedge, it is a correctness requirement, and
 * dropping it was a regression this hook was written twice to avoid.
 * `ModelAndProviderContext.changeModel` writes BOTH the session row
 * (`updateAgentProvider`) and the global default (`setConfigProvider`), so after
 * a per-chat model switch the daemon has them in step — while the client's copy
 * of the row, cached in the `/agent/resume` payload the chat stream holds, is
 * the value from BEFORE the switch. A composer that preferred the row on every
 * disagreement would answer a switch by showing the model the user had just
 * switched away from, until the window reloaded.
 *
 * When the selection is barred there is no such ambiguity: Gate A refuses to
 * bind it to this chat at all, so it provably is not what runs here, and the
 * row is the only binding that can be.
 *
 * ⚠ The residual case is deliberate and unchanged from `main`: a PRIVATE chat
 * bound to a different PRIVATE model than the selection still shows the
 * selection. It is stale in the same narrow way it has always been, it is not
 * what F2 reported, and fixing it needs the client's cached row to be
 * invalidated on a switch rather than a heuristic here.
 *
 * ⚠ The provider **display names** are resolved through `getProviders`, which
 * `ConfigContext` caches. `versa_azure` is not what that provider is called
 * anywhere else in the app. The selected provider's tier comes off the same
 * fetch, and off the ROW rather than `row.metadata` — `metadata.tier` is the
 * type-level claim and `resolved_tier` the instance-resolved one (DR-26);
 * reading the type-level field would mis-classify an `ollama` re-pointed off
 * this machine, the exact demotion the tier exists to catch.
 */
export function usePinnedModel(
  session: Session | undefined,
  reportedByTurn: PinnedModelView | undefined
): PinnedModelPresentation {
  const { getProviders } = useConfig();
  const { currentModel, currentProvider } = useModelAndProvider();
  const [displayNames, setDisplayNames] = useState<Record<string, string>>({});
  const [selectedTier, setSelectedTier] = useState<ProviderTier | undefined>(undefined);

  const binding = chatBinding(session, reportedByTurn);
  const chatTier = session?.privacy_tier;
  const bindingProvider = binding?.provider;
  const selectedProvider = currentProvider ?? undefined;

  useEffect(() => {
    // ⚠ Nothing is fetched for a chat that cannot be barred. This hook runs in
    // every chat, on every render of the composer, and only a PRIVATE one can
    // ever refuse the selection — so a public chat costs no catalog read and no
    // state update.
    if (chatTier !== 'private' || !selectedProvider) {
      setSelectedTier(undefined);
      return;
    }
    const wanted = [bindingProvider, selectedProvider].filter((id): id is string => !!id);
    let cancelled = false;
    (async () => {
      try {
        const rows = await getProviders(false);
        if (cancelled) return;
        // The tier and both display names come from ONE pass over ONE fetch, so
        // they cannot be taken from different reads of the catalog.
        const resolved: Record<string, string> = {};
        for (const id of wanted) {
          const name = rows.find((row) => row.name === id)?.metadata?.display_name;
          if (name) resolved[id] = name;
        }
        setDisplayNames((prev) => {
          const unchanged = wanted.every((id) => prev[id] === resolved[id]);
          return unchanged ? prev : { ...prev, ...resolved };
        });
        const selectedRow = rows.find((row) => row.name === selectedProvider);
        setSelectedTier(selectedRow ? readResolvedProviderTier(selectedRow) : undefined);
      } catch {
        // A catalog we cannot read leaves the tier unresolved, and unresolved
        // means "say nothing and change nothing": both the note and the chip
        // override claim to know that the selection is not what runs here.
        if (!cancelled) setSelectedTier(undefined);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [chatTier, getProviders, bindingProvider, selectedProvider]);

  if (!binding || !selectionBarredByPrivacy(chatTier, selectedTier)) {
    return { notice: null };
  }

  // The chip and gauge state the binding whether or not it is news; the
  // SENTENCE needs the selection to be nameable and different.
  const differs = bindingDiffersFromSelection(binding, {
    provider: currentProvider,
    model: currentModel,
  });

  return {
    effectiveModel: binding,
    notice: differs
      ? pinnedModelNotice(
          bindingLabel(displayNames[binding.provider], binding.model),
          bindingLabel(displayNames[selectedProvider!], currentModel!)
        )
      : null,
  };
}
