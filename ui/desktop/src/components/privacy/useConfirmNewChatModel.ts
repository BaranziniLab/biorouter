import { useCallback } from 'react';
import { useConfig } from '../ConfigContext';
import { useModelAndProvider, type AppModelSelection } from '../ModelAndProviderContext';
import { readResolvedProviderTier } from './useBoundProviderTier';
import { toastWarning } from '../../toasts';
import type { ProviderTier } from '../../api/types.gen';

export const NEW_CHAT_MODEL_CHANGED_TITLE = 'Message not sent';

/**
 * The toast for a send refused because the model on screen was not the model a
 * new chat would have started on.
 *
 * Written for a person, and it states the tier in words because the tier is
 * the reason this check exists: a window reading "Private model, UCSF" must not
 * start a chat on a public model with nothing said. It names what is true now,
 * what the window had been showing, and where the message went — and stops.
 */
export function newChatModelChangedMessage(
  shownModel: string,
  fresh: AppModelSelection,
  freshProviderName: string | null,
  freshTier: ProviderTier | undefined
): string {
  if (!fresh.model || !fresh.provider) {
    return (
      `No model is set for new chats any more — this window was still showing ${shownModel}. ` +
      'Your message is back in the composer.'
    );
  }
  const tier =
    freshTier === 'private'
      ? ', a private model'
      : freshTier === 'public'
        ? ', a public model'
        : '';
  return (
    `New chats now start on ${fresh.model} (${freshProviderName ?? fresh.provider}${tier}), ` +
    `not ${shownModel}, which this window was still showing. Your message is back in the ` +
    'composer, and the model shown below is the one it will use.'
  );
}

/**
 * F3 — the last look before a composer creates a NEW chat.
 *
 * `/agent/start` binds a new chat to whatever `BIOROUTER_PROVIDER` /
 * `BIOROUTER_MODEL` say on the daemon at that instant, and it takes no provider
 * of its own. Everything the composer states about the model — name, gauge,
 * cost, the "Private model, UCSF" padlock — comes from this window's copy of
 * those two keys. `ModelAndProviderContext` keeps that copy current: every
 * window hears every renderer write, and re-reads on focus. What it cannot hear
 * is a write from outside the renderer while this window keeps its focus — a
 * `biorouter configure` in the terminal docked inside this very window is the
 * ordinary case. So the send path asks once more, at the only moment the answer
 * decides anything.
 *
 * Resolves `true` to proceed. `false` means the window was stale: the fresh
 * selection is already on screen (the re-read published it), a toast says what
 * changed, and the caller returns `false` so `ChatInput` puts the text back.
 *
 * ⚠ It refuses only a KNOWN mismatch. Nothing named on screen yet (the first
 * frames after launch, or no model at all) and a read that failed both proceed
 * exactly as before: the first had no label to be wrong about, and the second
 * has no evidence — `createSession` reports a daemon that cannot answer.
 *
 * ⚠ Not a gate. The daemon classifies the chat by what it binds, correctly,
 * whatever this does; this only keeps the human's last look honest.
 */
export function useConfirmNewChatModel(): () => Promise<boolean> {
  const { currentModel, currentProvider, modelConfigStatus, syncAppModelSelection } =
    useModelAndProvider();
  const { getProviders } = useConfig();

  return useCallback(async () => {
    if (modelConfigStatus !== 'ready' || !currentProvider || !currentModel) return true;

    const fresh = await syncAppModelSelection();
    if (!fresh) return true;
    if (fresh.provider === currentProvider && fresh.model === currentModel) return true;

    let providerName: string | null = null;
    let tier: ProviderTier | undefined;
    if (fresh.provider) {
      try {
        const row = (await getProviders(false)).find(
          (candidate) => candidate.name === fresh.provider
        );
        providerName = row?.metadata?.display_name ?? null;
        tier = readResolvedProviderTier(row);
      } catch {
        // The sentence is still complete and still true without either.
      }
    }
    toastWarning({
      title: NEW_CHAT_MODEL_CHANGED_TITLE,
      msg: newChatModelChangedMessage(currentModel, fresh, providerName, tier),
    });
    return false;
  }, [currentModel, currentProvider, modelConfigStatus, syncAppModelSelection, getProviders]);
}
