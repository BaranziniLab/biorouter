import type { SessionClassification } from '../../api/types.gen';
import type { PinnedModelView } from '../../hooks/chatStreamStore';
import type { View, ViewOptions } from '../../utils/navigationUtils';
import ModelsBottomBar from '../settings/models/bottom_bar/ModelsBottomBar';
import { BottomMenuReasoningEffort } from './BottomMenuReasoningEffort';

/**
 * The composer's model and effort picker (spec 3.7): one chip naming the model
 * that answers the next message, with the reasoning effort chosen inside its
 * menu.
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
  /** Opens the provider catalog when no model is configured. */
  setView: (view: View, options?: ViewOptions) => void;
}

export function ModelEffortChip({
  sessionId,
  reasoningScope,
  effectiveModel,
  privacyTier,
  setView,
}: ModelEffortChipProps) {
  return (
    <div data-testid="composer-model-chip" className="flex min-w-0 items-center gap-2">
      <BottomMenuReasoningEffort scope={reasoningScope} />
      <div className="min-w-0">
        <ModelsBottomBar
          sessionId={sessionId}
          effectiveModel={effectiveModel}
          privacyTier={privacyTier}
          setView={setView}
          hideAlertPopover
        />
      </div>
    </div>
  );
}
