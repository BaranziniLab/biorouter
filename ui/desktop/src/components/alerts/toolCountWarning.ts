import { openCurrentChatExtensions } from '../../utils/sessionToolEvents';
import { AlertType, type Alert } from './types';

const TOOLS_MAX_SUGGESTED = 60;

export function toolCountWarning(toolCount: number | null, sessionId: string | null): Alert | null {
  if (toolCount === null || toolCount <= TOOLS_MAX_SUGGESTED || !sessionId) return null;
  return {
    type: AlertType.Warning,
    // The count is the one visible line; why it matters and what the defaults reach are the
    // info icon beside it (owner, 2026-11-13; design.md 9.16). "Review currently attached
    // extensions here" restated the action under it and is gone.
    message: `This chat can call ${toolCount} tools, including built-in capabilities (recommended: ${TOOLS_MAX_SUGGESTED} or fewer).`,
    help: 'Too many tools can degrade performance. Capability defaults apply to a new chat.',
    action: {
      text: 'Review extensions',
      onClick: () => {
        window.dispatchEvent(new CustomEvent('hide-alert-popover'));
        openCurrentChatExtensions(sessionId);
      },
    },
    autoShow: false,
  };
}
