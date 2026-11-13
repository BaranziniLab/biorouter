import type { SessionClassification } from '../../api/types.gen';
import type { PinnedModelView } from '../../hooks/chatStreamStore';
import type { PrivacyBasis } from '../../utils/sessionDecisions';
import type { View, ViewOptions } from '../../utils/navigationUtils';
import ModelsBottomBar from '../settings/models/bottom_bar/ModelsBottomBar';

/**
 * The composer's model and effort picker (spec 3.7): one chip naming the model
 * that answers the next message ("gpt-5.6-sol · Deep" and its padlock), with
 * the reasoning effort chosen inside its menu as Quick, Normal or Deep.
 *
 * ⚠ The props are the wave-0 contract (WS-PICKERS). `ChatInput` places this
 * inside the composer card, beside Send, and owns nothing below it.
 */
export interface ModelEffortChipProps {
  /** The focused chat, or `null` on Home and in a chat that has not started. */
  sessionId: string | null;
  /**
   * Where the effort is kept: `sessionReasoningScope(sessionId)` in a chat,
   * `draftReasoningScope(key)` before one (`store/reasoningEffort.ts`).
   */
  reasoningScope: string;
  /** The chat's own binding when it differs from the app-wide selection. */
  effectiveModel?: PinnedModelView;
  /** The chat's ratcheted classification (`ChatInput`'s `sessionPrivacyTier`). */
  privacyTier?: SessionClassification;
  /** DR-33: why the chat is public, when a decision model made it so. */
  privacyBasis?: PrivacyBasis | null;
  /** Opens the provider catalog when no model is configured. */
  setView: (view: View, options?: ViewOptions) => void;
  /**
   * The composer's dictation key, under which a draft dictated with private
   * terms eligible is flagged (DD-71): switching an unsent chat holding one to a
   * public model asks MSG-PRIVATE-DRAFT-SWITCH first.
   */
  dictationDraftKey?: string;
}

export function ModelEffortChip({
  sessionId,
  reasoningScope,
  effectiveModel,
  privacyTier,
  privacyBasis,
  setView,
  dictationDraftKey,
}: ModelEffortChipProps) {
  return (
    <div data-testid="composer-model-chip" className="flex min-w-0 items-center">
      <ModelsBottomBar
        sessionId={sessionId}
        effectiveModel={effectiveModel}
        privacyTier={privacyTier}
        privacyBasis={privacyBasis}
        setView={setView}
        reasoningScope={reasoningScope}
        dictationDraftKey={dictationDraftKey}
        hideAlertPopover
      />
    </div>
  );
}
