import { useState, useEffect, useRef } from 'react';
import { toolIdentifierToTitleCase } from '../utils';
import PermissionModal from './settings/permission/PermissionModal';
import { ChevronRight, Lock, Check, X, AlertTriangle } from './icons/app-icons';
import { confirmToolAction, ActionRequired } from '../api';
import { Button } from './ui/button';
import { ToolCallPreview, ToolRiskBadge } from './ToolCallPreview';
import { userActionHeaders } from '../utils/userAction';
import { isBrowserSurface } from '../utils/surface';

/**
 * What this card says instead of offering three buttons that cannot work.
 *
 * A `biorouter serve` daemon is started with `Stdio::null()` (SD-7), so no
 * proof-of-user digest is ever installed and `confirm_tool_action` refuses
 * every decision with `reason: "noKeyInstalled"` — not for this user, not for
 * this request, but for anyone, always. Rendering Allow/Deny there is a lie the
 * user only discovers by clicking.
 */
const BROWSER_CANNOT_APPROVE =
  'This page is served to a browser, which has no way to prove a decision came from you ' +
  'rather than from the model. Answer this request in the Biorouter desktop app.';

/** The refusal reason the daemon returns when no approval can ever be granted. */
function refusalIsPermanent(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'reason' in error &&
    (error as { reason?: unknown }).reason === 'noKeyInstalled'
  );
}

const ALLOW_ONCE = 'allow_once';
const ALWAYS_ALLOW = 'always_allow';
const DENY = 'deny';

// Global state to track tool confirmation decisions
// This persists across navigation within the same session
const toolConfirmationState = new Map<
  string,
  {
    clicked: boolean;
    status: string;
    actionDisplay: string;
  }
>();

type ToolConfirmationData = Extract<ActionRequired['data'], { actionType: 'toolConfirmation' }>;

/** `developer__text_editor` → `Text Editor`. */
function friendlyToolName(toolName: string): string {
  return toolIdentifierToTitleCase(toolName.split('__').pop() ?? toolName);
}

interface ToolConfirmationProps {
  sessionId: string;
  isCancelledMessage: boolean;
  isClicked: boolean;
  actionRequiredContent: ActionRequired & { type: 'actionRequired' };
}

export default function ToolConfirmation({
  sessionId,
  isCancelledMessage,
  isClicked,
  actionRequiredContent,
}: ToolConfirmationProps) {
  const data = actionRequiredContent.data as ToolConfirmationData;
  // BR-63: `risk` and `preview` are optional — a confirmation persisted before
  // BR-63, or replayed from an older daemon, simply has neither and the card
  // degrades to its pre-BR-63 shape rather than breaking.
  const { id: toolConfirmationId, toolName, prompt, risk, preview } = data;

  // Check if we have a stored state for this tool confirmation
  const storedState = toolConfirmationState.get(toolConfirmationId);

  // Initialize state from stored state if available, otherwise use props/defaults
  const [clicked, setClicked] = useState(storedState?.clicked ?? isClicked);
  const [status, setStatus] = useState(storedState?.status ?? 'unknown');
  const [actionDisplay, setActionDisplay] = useState(storedState?.actionDisplay ?? '');
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [isSending, setIsSending] = useState(false);
  const [confirmationError, setConfirmationError] = useState('');
  // Empty until we know approvals are impossible here — set up-front on a
  // browser surface, and by the daemon's own refusal reason anywhere else (a
  // desktop build talking to a daemon someone started by hand, say).
  const [cannotApprove, setCannotApprove] = useState(() =>
    isBrowserSurface() ? BROWSER_CANNOT_APPROVE : ''
  );
  const sendingRef = useRef(false);

  // Sync internal state with stored state and props
  useEffect(() => {
    const currentStoredState = toolConfirmationState.get(toolConfirmationId);

    // If we have stored state, use it
    if (currentStoredState) {
      setClicked(currentStoredState.clicked);
      setStatus(currentStoredState.status);
      setActionDisplay(currentStoredState.actionDisplay);
    } else if (isClicked && !clicked) {
      // Fallback to prop-based logic for historical confirmations
      setClicked(isClicked);
      if (status === 'unknown') {
        setStatus('confirmed');
        setActionDisplay('confirmed');

        // Store this state for future renders
        toolConfirmationState.set(toolConfirmationId, {
          clicked: true,
          status: 'confirmed',
          actionDisplay: 'confirmed',
        });
      }
    }
  }, [isClicked, clicked, status, toolName, toolConfirmationId]);

  const handleButtonClick = async (newStatus: string) => {
    if (sendingRef.current || clicked || isClicked || isCancelledMessage) return;
    sendingRef.current = true;
    setIsSending(true);
    setConfirmationError('');
    let newActionDisplay;

    if (newStatus === ALWAYS_ALLOW) {
      newActionDisplay = 'always allowed';
    } else if (newStatus === ALLOW_ONCE) {
      newActionDisplay = 'allowed once';
    } else if (newStatus === DENY) {
      newActionDisplay = 'denied';
    } else {
      newActionDisplay = 'denied';
    }

    try {
      const response = await confirmToolAction({
        headers: await userActionHeaders(),
        body: {
          sessionId: sessionId,
          id: toolConfirmationId,
          action: newStatus,
          principalType: 'Tool',
        },
      });
      const acknowledgement = response.data;
      const acknowledgedStatus =
        acknowledgement && typeof acknowledgement === 'object' && 'status' in acknowledgement
          ? acknowledgement.status
          : undefined;
      if (refusalIsPermanent(response.error)) {
        const explanation = (response.error as { error?: unknown }).error;
        setCannotApprove(typeof explanation === 'string' ? explanation : BROWSER_CANNOT_APPROVE);
        return;
      }
      if (
        response.error ||
        (acknowledgedStatus !== 'delivered' &&
          acknowledgedStatus !== 'already_resolved' &&
          acknowledgedStatus !== 'unknown')
      ) {
        setConfirmationError('Could not confirm your decision. Try again.');
        return;
      }
      const resolvedStatus = acknowledgedStatus === 'delivered' ? newStatus : acknowledgedStatus;
      if (acknowledgedStatus === 'already_resolved') newActionDisplay = 'already answered';
      if (acknowledgedStatus === 'unknown') newActionDisplay = 'no longer available';
      setClicked(true);
      setStatus(resolvedStatus);
      setActionDisplay(newActionDisplay);
      toolConfirmationState.set(toolConfirmationId, {
        clicked: true,
        status: resolvedStatus,
        actionDisplay: newActionDisplay,
      });
    } catch {
      setConfirmationError('Could not confirm your decision. Try again.');
    } finally {
      sendingRef.current = false;
      setIsSending(false);
    }
  };

  const handleModalClose = () => {
    setIsModalOpen(false);
  };

  function getExtensionName(toolName: string): string {
    const parts = toolName.split('__');
    return parts.length > 1 ? parts[0] : '';
  }

  // `prompt` is `approval_prompt_for_request` — every reason an INSPECTOR gave
  // for escalating this call — so its presence is what marks a security finding.
  //
  // One derived boolean rather than two independent reads of `prompt`: the
  // warning banner and the withheld "Always Allow" are two halves of one
  // decision, and a card that shows the banner while still offering a permanent
  // grant (or the reverse) is worse than either. Whitespace is not a finding —
  // a blank prompt used to paint an empty warning band *and* take the user's
  // "Always Allow" away.
  //
  // ⚠ It is also why nothing but an inspector may write that field. The
  // coding-agent bridge used to put its own framing there ("<child> asked to run
  // this through Biorouter"), which put every bridged call behind a warning
  // banner with no way to grant a lasting permission. The card was reading the
  // field correctly; the producer was misusing it. See `bridge.rs::await_approval`.
  const securityFinding = typeof prompt === 'string' && prompt.trim().length > 0;

  // One cohesive, bordered "permission request" card. A single border wraps the
  // whole element (header + actions) so there are no mismatched borders, it uses
  // the app's standard card tokens + typography, and a gentle slide-in makes it
  // read as a distinct prompt the user is meant to act on.
  return isCancelledMessage ? (
    <div className="biorouter-message-content rounded-2xl border border-border-subtle bg-background-muted px-4 py-3 text-sm text-text-muted">
      Tool call confirmation was canceled.
    </div>
  ) : (
    <>
      <div className="biorouter-message-content text-body overflow-hidden rounded-2xl border border-border-subtle bg-background-default animate-in fade-in slide-in-from-bottom-1 duration-200">
        {/* Security finding banner, only when an inspector flagged one */}
        {securityFinding && (
          <div
            data-testid="tool-security-finding"
            className="flex items-start gap-2 border-b border-border-subtle bg-background-warning/10 px-4 py-2.5 text-sm text-text-warning"
          >
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <span>{prompt}</span>
          </div>
        )}

        {clicked ? (
          // Resolved state — one consistent row inside the same card.
          <div className="flex items-center justify-between px-4 py-3">
            <div className="flex items-center gap-2 text-sm text-text-default">
              {status === 'deny' || status === 'unknown' || status === 'already_resolved' ? (
                <X className="h-4 w-4 shrink-0 text-text-muted" />
              ) : (
                <Check className="h-4 w-4 shrink-0 text-text-muted" />
              )}
              <span>
                {isClicked
                  ? 'Tool confirmation is not available'
                  : `${friendlyToolName(toolName)} is ${actionDisplay}`}
              </span>
            </div>
            <button
              type="button"
              className="flex items-center gap-1 text-sm text-text-muted transition-colors hover:text-text-default"
              onClick={() => setIsModalOpen(true)}
            >
              Change
              <ChevronRight className="h-4 w-4" />
            </button>
          </div>
        ) : (
          // Pending state — the agent is asking the user for permission.
          <div className="px-4 py-3">
            <div className="mb-3 flex flex-wrap items-center gap-2">
              <Lock className="h-4 w-4 shrink-0 text-text-muted" />
              <span className="text-sm font-medium text-text-default">
                {/* Name the tool. "this tool" told the user nothing.
                    ⚠ NOT `font-mono`. `friendlyToolName` returns a Title Case
                    display name ("Install Extension"), not the raw
                    `extensionmanager__install_extension` id it started life as —
                    and the resolved state a few lines above renders the SAME
                    string in the body font. One string, two typefaces, in one
                    component. Monospace here is a leftover from when this
                    printed the identifier.
                    The <span> stays: it keeps the name a distinct node, which is
                    what lets a test assert on the name alone rather than on the
                    whole "Run … ?" sentence. Only the font moved. */}
                Run <span>{friendlyToolName(toolName)}</span>?
              </span>
              {risk && <ToolRiskBadge risk={risk} />}
            </div>

            {/* BR-63: the whole point — what the call will actually do. */}
            {preview && (
              <div className="mb-3">
                <ToolCallPreview preview={preview} />
              </div>
            )}

            {cannotApprove ? (
              <p
                role="status"
                className="rounded-lg bg-background-muted px-3 py-2 text-sm text-text-muted"
              >
                {cannotApprove}
              </p>
            ) : (
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  type="button"
                  size="sm"
                  variant="default"
                  disabled={isSending}
                  onClick={() => handleButtonClick(ALLOW_ONCE)}
                >
                  Allow Once
                </Button>
                {/* Only offer "Always Allow" when there's no security finding. A
                    permanent grant is not something to decide from a card that
                    exists because an inspector objected. */}
                {!securityFinding && (
                  <Button
                    type="button"
                    size="sm"
                    variant="secondary"
                    disabled={isSending}
                    onClick={() => handleButtonClick(ALWAYS_ALLOW)}
                  >
                    Always Allow
                  </Button>
                )}
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={isSending}
                  onClick={() => handleButtonClick(DENY)}
                >
                  Deny
                </Button>
              </div>
            )}
            {confirmationError && (
              <p role="alert" className="mt-2 text-sm text-text-warning">
                {confirmationError}
              </p>
            )}
          </div>
        )}
      </div>

      {/* Modal for updating tool permission */}
      {isModalOpen && (
        <PermissionModal onClose={handleModalClose} extensionName={getExtensionName(toolName)} />
      )}
    </>
  );
}
