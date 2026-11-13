import { useState, useEffect, useRef } from 'react';
import PermissionModal from './settings/permission/PermissionModal';
import { Check, Clock, X, AlertTriangle } from './icons/app-icons';
import { confirmToolAction, ActionRequired } from '../api';
import { Button } from './ui/button';
import { ToolCallPreview, ToolRiskBadge } from './ToolCallPreview';
import { TranscriptRow } from './TranscriptRow';
import {
  APPROVAL_COPY,
  CREW_READ_CONSENT_EVENT,
  isCrewPersonTool,
  isCrewReadConsent,
  sentenceStart,
  toolCallPhrase,
} from './toolCallCopy';
import { userActionHeaders } from '../utils/userAction';
import { isBrowserSurface } from '../utils/surface';
import {
  FOLDER_ACCESS_COPY,
  folderShown,
  requesterOnPreview,
  useFolderRequester,
} from './folderAccessCopy';

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

/** A leading warning sign (with or without its emoji selector) and the space after it. */
const LEADING_WARNING_GLYPH = /^\s*\u26A0\uFE0F?\s*/u;

/**
 * The finding as the card shows it, beside its own warning icon.
 *
 * A producer may lead its prompt with "⚠ ", because the terminal prints the
 * prompt as bare text and that glyph is its only warning sign there:
 * `bug_report::private_warning` does ("⚠ This chat is PRIVATE: …"). Drawn
 * after the card's icon it made two warning signs side by side (final check,
 * 2026-10-12), so the card drops a LEADING one and keeps the rest of the text
 * exactly: a ⚠ inside the sentence marks a second warning and stays. Stripping
 * here rather than at the producer also fixes cards persisted by an older
 * daemon. A prompt that is only the glyph keeps it, rather than going blank.
 */
function findingText(prompt: string): string {
  const stripped = prompt.replace(LEADING_WARNING_GLYPH, '');
  return stripped.length > 0 ? stripped : prompt;
}

const ALLOW_ONCE = 'allow_once';
const ALWAYS_ALLOW = 'always_allow';
const ALLOW_SESSION = 'allow_session';
const DENY = 'deny';
const CANCEL = 'cancel';

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

/**
 * `developer__text_editor` → "text editor", for the middle of a sentence ("Run
 * text editor?"). Sentence case: acronyms and product names keep capitals.
 */
function friendlyToolName(toolName: string): string {
  return toolCallPhrase(toolName);
}

/** The status a card holds once its answer window closed unanswered. */
const EXPIRED = 'expired';

interface ToolConfirmationProps {
  sessionId: string;
  isCancelledMessage: boolean;
  isClicked: boolean;
  /**
   * The daemon reported that this card's answer window closed before anyone
   * answered it (an inline notice carrying `expiredActionId`, live or in a
   * replay). Nothing waits on the card any more, so it shows that instead of
   * buttons.
   */
  isExpired?: boolean;
  actionRequiredContent: ActionRequired & { type: 'actionRequired' };
}

export default function ToolConfirmation({
  sessionId,
  isCancelledMessage,
  isClicked,
  isExpired = false,
  actionRequiredContent,
}: ToolConfirmationProps) {
  const data = actionRequiredContent.data as ToolConfirmationData;
  // BR-63: `risk` and `preview` are optional — a confirmation persisted before
  // BR-63, or replayed from an older daemon, simply has neither and the card
  // degrades to its pre-BR-63 shape rather than breaking.
  const { id: toolConfirmationId, toolName, prompt, risk, preview } = data;
  const folderAccess = preview?.kind === 'folderAccess';
  // Crew's person tools (AO-12): each card is answered for one action, and no standing permission
  // applies, so there is nothing to Change. Their prompt is a summary line, then a line per fact.
  const crewPerson = isCrewPersonTool(toolName);
  // The read consent lasts until the person takes it back: its button and answered row say so,
  // never "once".
  const readConsent = isCrewReadConsent(toolName, data.arguments, prompt);
  const consentWorkspace =
    readConsent && typeof data.arguments?.workspace === 'string' && data.arguments.workspace
      ? data.arguments.workspace
      : null;
  // Who asks: the daemon's name on the card (a subagent's card shown in the chat
  // that started it), else the tab's own subagent, else this chat's agent.
  const tabRequester = useFolderRequester();
  const folderRequester = folderAccess ? (requesterOnPreview(preview) ?? tabRequester) : null;
  const browserFolderAccess = folderAccess && isBrowserSurface();
  const [folderApprovalKey, setFolderApprovalKey] = useState('');

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
    isBrowserSurface() && !folderAccess ? BROWSER_CANNOT_APPROVE : ''
  );
  const sendingRef = useRef(false);
  // Expired by the daemon's notice, or learned from a late click that found
  // nothing waiting (`status: unknown`, kept in the module map so a remount
  // still knows).
  const expired = !clicked && (isExpired || status === EXPIRED);

  useEffect(() => {
    if (isCancelledMessage || isClicked || clicked || expired) setFolderApprovalKey('');
  }, [isCancelledMessage, isClicked, clicked, expired]);

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
    if (sendingRef.current || clicked || isClicked || isCancelledMessage || expired) return;
    sendingRef.current = true;
    setIsSending(true);
    setConfirmationError('');
    let newActionDisplay;

    if (newStatus === ALLOW_SESSION) {
      newActionDisplay = 'allowed for this session';
    } else if (newStatus === ALWAYS_ALLOW) {
      newActionDisplay = APPROVAL_COPY.outcome.alwaysAllow;
    } else if (newStatus === ALLOW_ONCE) {
      newActionDisplay = APPROVAL_COPY.outcome.allowOnce;
    } else if (newStatus === CANCEL) {
      newActionDisplay = 'cancelled';
    } else {
      newActionDisplay = APPROVAL_COPY.outcome.deny;
    }

    try {
      const response = await confirmToolAction({
        headers: {
          ...(await userActionHeaders()),
          ...(browserFolderAccess && folderApprovalKey
            ? { 'X-Folder-Access-Key': folderApprovalKey }
            : {}),
        },
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
      if (folderAccess && response.error) {
        const explanation =
          typeof response.error === 'object' && response.error !== null && 'error' in response.error
            ? response.error.error
            : undefined;
        setConfirmationError(
          typeof explanation === 'string'
            ? explanation
            : 'Folder approval was refused. Check the separate approval key and try again; this operation remains pending.'
        );
        return;
      }
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
      if (acknowledgedStatus === 'unknown') {
        // Nothing waits on this card any more: its window closed between the
        // notice and the click, or the notice never reached this view. The
        // decision was dropped, so the card says what the notice would have.
        setStatus(EXPIRED);
        toolConfirmationState.set(toolConfirmationId, {
          clicked: false,
          status: EXPIRED,
          actionDisplay: '',
        });
        return;
      }
      const resolvedStatus = acknowledgedStatus === 'delivered' ? newStatus : acknowledgedStatus;
      if (acknowledgedStatus === 'already_resolved')
        newActionDisplay = APPROVAL_COPY.outcome.alreadyResolved;
      setClicked(true);
      setStatus(resolvedStatus);
      setActionDisplay(newActionDisplay);
      toolConfirmationState.set(toolConfirmationId, {
        clicked: true,
        status: resolvedStatus,
        actionDisplay: newActionDisplay,
      });
      // The chat now holds a read consent: its Crew bar shows it, with Revoke (AO-12).
      if (readConsent && resolvedStatus === ALLOW_ONCE)
        window.dispatchEvent(
          new CustomEvent(CREW_READ_CONSENT_EVENT, { detail: { sessionId: sessionId } })
        );
    } catch {
      setConfirmationError(APPROVAL_COPY.confirmFailed);
    } finally {
      setFolderApprovalKey('');
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
  // warning banner and the withheld "Always allow" are two halves of one
  // decision, and a card that shows the banner while still offering a permanent
  // grant (or the reverse) is worse than either. Whitespace is not a finding —
  // a blank prompt used to paint an empty warning band *and* take the user's
  // "Always allow" away.
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
  // banner and still withhold "Always allow" —
  // `extension_manager_extension.rs` (install, delete), `platform_approval.rs`,
  // `skills_extension.rs` and `bug_report/mod.rs`. Withholding the grant is
  // arguably wanted for the destructive ones (they carry `requires_user_proof`),
  // but the *banner* is not: one of them reads "install … from the trusted BAAM
  // registry" under a warning triangle. Closing that needs a field distinct from
  // `prompt`, which is a protocol change, so it is a known gap rather than a fix.
  //
  // A folder card's `prompt` is not a finding either: it is
  // `FolderAccessRequest::prompt`, the terminal's wording of the same question,
  // tool id and all ("… for developer__text_editor?"). The card asks it in its
  // own line and keeps the terms under Details (R2-1), so drawing it as well
  // asked the question twice, and kept the whole warning on the answered row
  // (R2-3).
  const securityFinding = !folderAccess && typeof prompt === 'string' && prompt.trim().length > 0;

  // The finding is a security fact, so it stays visible on every state of the
  // card: a warning line, not a tinted band (a band inside a card is a box in
  // a box).
  const findingLine = securityFinding ? (
    <div
      data-testid="tool-security-finding"
      className="flex items-start gap-2 text-body text-text-warning"
    >
      <AlertTriangle aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
      {/* A Crew card's summary and the lines under it are separate lines (live QA: "as you It
          can then read…" ran together). */}
      <span className={crewPerson ? 'min-w-0 whitespace-pre-line' : 'min-w-0'}>
        {findingText(prompt)}
      </span>
    </div>
  ) : null;

  // Expired: the window closed before anyone answered, so the call gave up and
  // the tool did not run. One calm row in the transcript's muted ink (Red is
  // rare, design.md 9.14): not a failure, not a warning, and no control,
  // because the way forward is to ask again in the chat.
  if (expired && !isClicked) {
    return (
      <div className="biorouter-message-content text-body" data-testid="tool-approval-expired">
        <TranscriptRow icon={Clock} label={APPROVAL_COPY.expired} />
      </div>
    );
  }

  if (isCancelledMessage && !clicked && !isClicked) {
    return (
      <div className="biorouter-message-content text-body">
        <TranscriptRow icon={X} label={APPROVAL_COPY.canceled} />
      </div>
    );
  }

  // Answered: the card has done its job, so it collapses to a transcript row
  // with the outcome and a way to change the standing permission.
  if (clicked) {
    const declined = status === DENY || status === CANCEL || status === 'already_resolved';
    return (
      <>
        <div className="biorouter-message-content text-body flex min-w-0 flex-col gap-1">
          <TranscriptRow
            icon={declined ? X : Check}
            label={
              isClicked
                ? APPROVAL_COPY.unavailable
                : readConsent && status === ALLOW_ONCE
                  ? APPROVAL_COPY.readConsent.allowed(consentWorkspace)
                  : APPROVAL_COPY.resolved(sentenceStart(friendlyToolName(toolName)), actionDisplay)
            }
            trailing={
              // The row's own voice (13px muted): a 14px control beside a 13px
              // line read as a second size on one line.
              !folderAccess &&
              !crewPerson && (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="text-secondary text-text-muted"
                  onClick={() => setIsModalOpen(true)}
                >
                  {APPROVAL_COPY.change}
                </Button>
              )
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

  // Asking: the one card recipe for anything that needs the person to act:
  // radius 12, the subtle fill with no outline, 16px in. Title, what the call
  // will do, then the decision at the bottom right.
  return (
    <>
      <div className="biorouter-message-content br-action-card br-fill-surface br-enter text-body">
        {findingLine}

        <div className="flex flex-wrap items-center gap-2">
          <span className="text-label text-text-default">
            {/* Name the tool. "this tool" told the user nothing.
                ⚠ NOT `font-mono`. `friendlyToolName` returns a display name
                ("install extension"), not the raw
                `extensionmanager__install_extension` id it started life as, and
                the answered row renders the SAME string in the body font. The
                <span> keeps the name a distinct node, which is what lets a test
                assert on the name alone rather than on the whole sentence. */}
            {preview?.kind === 'folderAccess' ? (
              <span data-testid="folder-access-question">
                {FOLDER_ACCESS_COPY.question({
                  requester: folderRequester,
                  access: preview.access,
                  folder: folderShown(preview.folder, preview.files),
                })}
              </span>
            ) : (
              <>
                {APPROVAL_COPY.question} <span>{friendlyToolName(toolName)}</span>?
              </>
            )}
          </span>
          {risk && <ToolRiskBadge risk={risk} />}
        </div>

        {/* BR-63: the whole point — what the call will actually do. */}
        {preview && <ToolCallPreview preview={preview} />}

        {browserFolderAccess && (
          <div className="flex flex-col gap-1 text-body">
            <label htmlFor={`${toolConfirmationId}-folder-key`}>Folder approval key</label>
            <input
              id={`${toolConfirmationId}-folder-key`}
              type="password"
              autoComplete="off"
              value={folderApprovalKey}
              onChange={(event) => setFolderApprovalKey(event.target.value)}
              className="rounded border border-border-default bg-background-default px-2 py-1"
              aria-describedby={`${toolConfirmationId}-folder-key-help`}
            />
            <span id={`${toolConfirmationId}-folder-key-help`} className="text-text-muted">
              Use the separate key set with biorouter serve --folder-access-approval. It is cleared
              after each decision and grants no other protected permission. If setup was omitted,
              restart the server with that flag, then retry the file operation.
            </span>
          </div>
        )}
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
              disabled={isSending || (browserFolderAccess && !folderApprovalKey)}
              onClick={() => handleButtonClick(ALLOW_ONCE)}
            >
              {readConsent ? APPROVAL_COPY.readConsent.allow : APPROVAL_COPY.allowOnce}
            </Button>
            {/* Only offer "Always allow" when there's no security finding. A
                permanent grant is not something to decide from a card that
                exists because an inspector objected. */}
            {(folderAccess || !securityFinding) && (
              <Button
                type="button"
                size="sm"
                variant="secondary"
                disabled={isSending || (browserFolderAccess && !folderApprovalKey)}
                onClick={() => handleButtonClick(folderAccess ? ALLOW_SESSION : ALWAYS_ALLOW)}
              >
                {folderAccess ? FOLDER_ACCESS_COPY.allowSession : APPROVAL_COPY.alwaysAllow}
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
            {folderAccess && (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                disabled={isSending}
                onClick={() => handleButtonClick(CANCEL)}
              >
                {FOLDER_ACCESS_COPY.cancel}
              </Button>
            )}
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
