import { MonitorsDialog } from './MonitorsDialog';
import { chatModelKey, type DraftDecisionSettings } from './draftDecisionSettings';
import type { DecisionSettings } from '../../utils/sessionDecisions';

/** One empty record, so an empty draft is the same value on every render. */
const NO_SETTINGS: DecisionSettings = { defaultModel: null, monitors: [] };

/**
 * The Monitors dialog for a chat that has no message yet (DR-33, design §4), shared
 * by both composers (Home's `Hub` and a blank chat's `BaseChat`), which would
 * otherwise diverge silently. It reads and writes the composer's held draft; the
 * composer sends it as `decisionSettings` on `/agent/start` with the first message.
 *
 * `provider`/`model` are the model the chat will start on: each listed model then
 * carries what it would do to that chat, and an acceptance of "This chat will be
 * public" is held under it, so a model changed afterwards asks again at Send.
 */
export function DraftMonitorsDialog({
  open,
  onOpenChange,
  provider,
  model,
  chatModelName,
  draft,
  onDraftChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  provider: string | null;
  model: string | null;
  /** The chat model in words, for the tip. */
  chatModelName?: string | null;
  draft: DraftDecisionSettings | null;
  onDraftChange: (draft: DraftDecisionSettings | null) => void;
}) {
  const key = chatModelKey(provider, model);
  return (
    <MonitorsDialog
      open={open}
      onOpenChange={onOpenChange}
      sessionId=""
      // An unsent chat is classified Public until its first turn; the catalog's own
      // reasons and `chatEffect` say what each model would do to it.
      chatTier="public"
      settings={draft?.settings ?? NO_SETTINGS}
      chatModelName={chatModelName ?? model}
      draft={{
        chat: provider && model ? { boundProvider: provider, boundModel: model } : null,
        accepted: key !== null && draft?.acceptedFor === key,
        onChange: (settings, acceptedPublic) =>
          onDraftChange({ settings, acceptedFor: acceptedPublic ? key : null }),
      }}
    />
  );
}
