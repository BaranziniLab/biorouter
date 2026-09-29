import { useEffect, useState } from 'react';
import type { Session } from '../../../api';
import { subscribeSessionBindingChanges } from '../../../utils/sessionBindingSync';
import { cacheGet } from '../../../utils/sessionNameSync';
import type { ModelChoice } from '../pane/useConfiguredModels';

/** The model a chat's row names, or `null` when it names none. */
function modelOf(
  session: Pick<Session, 'provider_name' | 'model_config'> | null | undefined
): ModelChoice | null {
  const provider = session?.provider_name?.trim();
  const model = session?.model_config?.model_name?.trim();
  return provider && model ? { provider, model } : null;
}

/**
 * The model a chat is bound to, as this window knows its row: what the daemon binds Crew access to
 * when the chat is granted (`Agent::provider_for_crew_grant`, which prefers the chat's live agent
 * and falls back to this row). Read from the chat store's cache, which the chat's own controller
 * keeps fresh, and followed through its binding announcements (`sessionBindingSync`), so a model
 * switched in the chat is the model named here. Never fetched: a chat this window has not loaded
 * is `null`.
 *
 * Presentation only (AG-F1, SF-F5): the Chat access pane names the model and says early what the
 * daemon would refuse. `null` — unknown, or a row that names no model — and the pane says nothing
 * about the model: the daemon still decides every grant.
 */
export function useChatModel(sessionId: string | null): ModelChoice | null {
  const [model, setModel] = useState<{ sessionId: string; choice: ModelChoice | null } | null>(
    () => (sessionId ? { sessionId, choice: modelOf(cacheGet(sessionId)?.session) } : null)
  );

  useEffect(() => {
    if (!sessionId) return;
    setModel((current) =>
      current?.sessionId === sessionId
        ? current
        : { sessionId, choice: modelOf(cacheGet(sessionId)?.session) }
    );
    return subscribeSessionBindingChanges((change) => {
      if (change.sessionId === sessionId && change.provider && change.model)
        setModel({ sessionId, choice: { provider: change.provider, model: change.model } });
    });
  }, [sessionId]);

  return sessionId && model?.sessionId === sessionId ? model.choice : null;
}
