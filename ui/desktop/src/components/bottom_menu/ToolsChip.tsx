import type { SessionClassification } from '../../api/types.gen';
import { BottomMenuExtensionSelection } from './BottomMenuExtensionSelection';
import { BottomMenuKnowledgeSelection } from './BottomMenuKnowledgeSelection';
import { BottomMenuSkillSelection } from './BottomMenuSkillSelection';

/**
 * The composer's Tools chip (spec 3.7): one control, labelled "Tools" and the
 * sum of the extensions, skills and knowledge bases this chat can reach. It
 * opens one popover with Extensions, Skills and Knowledge.
 *
 * ⚠ The props are the wave-0 contract (WS-PICKERS). `ChatInput` places this
 * inside the composer card and owns nothing below it.
 */
export interface ToolsChipProps {
  /** The focused chat, or `null` on Home and in a chat that has not started. */
  sessionId: string | null;
  /**
   * The chat's ratcheted classification (`ChatInput`'s `sessionPrivacyTier`).
   * Not what an extension pairing is judged on: that is the bound model's tier,
   * which the picker reads itself (issue #56).
   */
  privacyTier?: SessionClassification;
}

export function ToolsChip({ sessionId, privacyTier }: ToolsChipProps) {
  return (
    <div data-testid="composer-tools-chip" className="flex flex-shrink-0 items-center gap-2">
      <BottomMenuExtensionSelection sessionId={sessionId} privacyTier={privacyTier} />
      <BottomMenuSkillSelection sessionId={sessionId} />
      <BottomMenuKnowledgeSelection />
    </div>
  );
}
