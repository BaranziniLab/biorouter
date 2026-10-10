import { useState, useEffect, useRef } from 'react';
import { toolIdentifierToTitleCase } from '../utils';
import PermissionModal from './settings/permission/PermissionModal';
import { Check, X, AlertTriangle } from './icons/app-icons';
import { confirmToolAction, ActionRequired } from '../api';
import { Button } from './ui/button';
import { ToolCallPreview, ToolRiskBadge } from './ToolCallPreview';
import { TranscriptRow } from './TranscriptRow';
import { APPROVAL_COPY } from './toolCallCopy';
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
const BROWSER_CANNOT_APPROVE = APPROVAL_COPY.browserCannotApprove;

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
      newActionDisplay = APPROVAL_COPY.outcome.alwaysAllow;
    } else if (newStatus === ALLOW_ONCE) {
      newActionDisplay = APPROVAL_COPY.outcome.allowOnce;
    } else {
      newActionDisplay = APPROVAL_COPY.outcome.deny;
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
        setConfirmationError(APPROVAL_COPY.confirmFailed);
        return;
      }
      const resolvedStatus = acknowledgedStatus === 'delivered' ? newStatus : acknowledgedStatus;
      if (acknowledgedStatus === 'already_resolved')
        newActionDisplay = APPROVAL_COPY.outcome.alreadyResolved;
      if (acknowledgedStatus === 'unknown') newActionDisplay = APPROVAL_COPY.outcome.unknown;
      setClicked(true);
      setStatus(resolvedStatus);
      setActionDisplay(newActionDisplay);
      toolConfirmationState.set(toolConfirmationId, {
        clicked: true,
        status: resolvedStatus,
        actionDisplay: newActionDisplay,
      });
    } catch {
      setConfirmationError(APPROVAL_COPY.confirmFailed);
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
  // ⚠ The coding-agent bridge used to put its own framing there ("<child> asked
  // to run this through Biorouter"), which put every bridged call behind a
  // warning banner with no way to grant a lasting permission. The card was
  // reading the field correctly; the producer was misusing it. Fixed in
  // `bridge.rs::await_approval` and pinned by `bridgeApprovalPrompt.test.ts`.
  //
  // ⚠ That guard covers `bridge.rs` and ONLY `bridge.rs` — it string-matches that
  // one file. Do NOT read it as "nothing but an inspector writes `prompt`": five
  // other production sites still put framing there, so their cards still draw the
  // banner and still withhold "Always Allow" —
  // `extension_manager_extension.rs` (install, delete), `platform_approval.rs`,
  // `skills_extension.rs` and `bug_report/mod.rs`. Withholding the grant is
  // arguably wanted for the destructive ones (they carry `requires_user_proof`),
  // but the *banner* is not: one of them reads "install … from the trusted BAAM
  // registry" under a warning triangle. Closing that needs a field distinct from
  // `prompt`, which is a protocol change, so it is a known gap rather than a fix.
  const securityFinding = typeof prompt === 'string' && prompt.trim().length > 0;

  // The finding is a security fact, so it stays visible on every state of the
  // card: a warning line, not a tinted band (a band inside a card is a box in
  // a box).
  const findingLine = securityFinding ? (
    <div
      data-testid="tool-security-finding"
      className="flex items-start gap-2 text-body text-text-warning"
    >
      <AlertTriangle aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
      <span className="min-w-0">{prompt}</span>
    </div>
  ) : null;

  if (isCancelledMessage) {
    return (
      <div className="biorouter-message-content text-body">
        <TranscriptRow icon={X} label={APPROVAL_COPY.canceled} />
      </div>
    );
  }

  // Answered: the card has done its job, so it collapses to a transcript row
  // with the outcome and a way to change the standing permission.
  if (clicked) {
    const declined = status === 'deny' || status === 'unknown' || status === 'already_resolved';
    return (
      <>
        <div className="biorouter-message-content text-body flex min-w-0 flex-col gap-1">
          <TranscriptRow
            icon={declined ? X : Check}
            label={
              isClicked
                ? APPROVAL_COPY.unavailable
                : APPROVAL_COPY.resolved(friendlyToolName(toolName), actionDisplay)
            }
            trailing={
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="text-text-muted"
                onClick={() => setIsModalOpen(true)}
              >
                {APPROVAL_COPY.change}
              </Button>
            }
          />
          {findingLine && <div className="pl-6">{findingLine}</div>}
        </div>
        {isModalOpen && (
          <PermissionModal onClose={handleModalClose} extensionName={getExtensionName(toolName)} />
        )}
      </>
    );
  }

  // Asking: the one card recipe for anything that needs the person to act —
  // radius 12, a hairline, the default ground, 16px in. Title, what the call
  // will do, then the decision at the bottom right.
  return (
    <>
      <div className="biorouter-message-content br-enter text-body flex min-w-0 flex-col gap-3 rounded-container border border-border-subtle bg-background-default p-4">
        {findingLine}

        <div className="flex flex-wrap items-center gap-2">
          <span className="text-label text-text-default">
            {/* Name the tool. "this tool" told the user nothing.
                ⚠ NOT `font-mono`. `friendlyToolName` returns a display name
                ("Install Extension"), not the raw
                `extensionmanager__install_extension` id it started life as, and
                the answered row renders the SAME string in the body font. The
                <span> keeps the name a distinct node, which is what lets a test
                assert on the name alone rather than on the whole sentence. */}
            {APPROVAL_COPY.question} <span>{friendlyToolName(toolName)}</span>?
          </span>
          {risk && <ToolRiskBadge risk={risk} />}
        </div>

        {/* BR-63: the whole point — what the call will actually do. */}
        {preview && <ToolCallPreview preview={preview} />}

        {cannotApprove ? (
          // A refusal, so it stays visible (principle 2).
          <p role="status" className="text-body text-text-muted">
            {cannotApprove}
          </p>
        ) : (
          <div className="flex flex-wrap items-center justify-end gap-2">
            <Button
              type="button"
              size="sm"
              variant="default"
              disabled={isSending}
              onClick={() => handleButtonClick(ALLOW_ONCE)}
            >
              {APPROVAL_COPY.allowOnce}
            </Button>
            {/* Only offer "Always allow" when there's no security finding. A
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
                {APPROVAL_COPY.alwaysAllow}
              </Button>
            )}
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={isSending}
              onClick={() => handleButtonClick(DENY)}
            >
              {APPROVAL_COPY.deny}
            </Button>
          </div>
        )}
        {confirmationError && (
          <p role="alert" className="text-supporting text-text-danger">
            {confirmationError}
          </p>
        )}
      </div>

      {/* Modal for updating tool permission */}
      {isModalOpen && (
        <PermissionModal onClose={handleModalClose} extensionName={getExtensionName(toolName)} />
      )}
    </>
  );
}
