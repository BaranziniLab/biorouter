import { useState, useEffect, useId, useRef } from 'react';
import type { ComponentProps } from 'react';
import PermissionModal from './settings/permission/PermissionModal';
import { Check, Clock, X, AlertTriangle } from './icons/app-icons';
import { confirmToolAction, ActionRequired } from '../api';
import { Button } from './ui/button';
import { folderAccessHelp, ToolCallPreview, ToolRiskBadge } from './ToolCallPreview';
import { Disclosure } from './ui/disclosure';
import { InfoTip } from './ui/info-tip';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/Tooltip';
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
import { fitsInfoTip, foldForHelp } from '../utils/proseFold';

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

/** The browser folder-key field's help, behind its info icon. */
const FOLDER_KEY_HELP =
  'The separate key set with biorouter serve --folder-access-approval, cleared after each decision. Without it, restart the server with that flag and retry.';

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

/**
 * The finding beside its warning triangle, folded to the one line a person needs (owner,
 * 2026-11-13: no solid chunks of text; design.md 9.14).
 *
 * `approval_prompt_for_request` joins every inspector's reason with a blank line, and a reason
 * can run to several sentences. The first sentence of the first reason is the security fact and
 * stays on the line. A short rest of that reason goes behind the info icon beside it; anything
 * longer, and every further reason, goes into a Details disclosure under the line, in order and
 * word for word, so nothing the inspectors said is lost.
 */
function FindingLine({ prompt }: { prompt: string }) {
  const text = findingText(prompt);
  const reasons = text
    .split(/\n\s*\n/)
    .map((reason) => reason.trim())
    .filter(Boolean);
  const [first = text, ...others] = reasons;
  const { lead, tip, details } = foldForHelp(first);
  const inlineTip = tip && others.length === 0 ? tip : '';
  const folded = [inlineTip ? '' : tip, details, ...others].filter(Boolean);
  return (
    <div data-testid="tool-security-finding" className="flex min-w-0 flex-col gap-1">
      <div className="flex items-start gap-2 text-body text-text-default">
        <AlertTriangle aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-text-warning" />
        <span className="min-w-0">
          {lead}
          {inlineTip ? (
            <>
              {' '}
              <InfoTip label={APPROVAL_COPY.findingName} help={inlineTip} />
            </>
          ) : null}
        </span>
      </div>
      {folded.length > 0 && (
        <div className="pl-6">
          <Disclosure
            label={
              others.length > 0
                ? APPROVAL_COPY.findingReasons(reasons.length)
                : APPROVAL_COPY.findingDetails
            }
          >
            <div className="flex flex-col gap-2 text-body text-text-default">
              {folded.map((part, index) => (
                <p key={index} className="min-w-0 [overflow-wrap:anywhere]">
                  {part}
                </p>
              ))}
            </div>
          </Disclosure>
        </div>
      )}
    </div>
  );
}

/**
 * A Crew person card's finding (AO-12): the daemon writes a summary line ("Post this in #general in
 * lab on leo as you"), then a line per fact (who it affects, what cannot be undone, a privacy
 * crossing). The summary is the line beside the triangle. The first fact's first sentence stays on
 * its own line under it, because it is usually the consequence the person is deciding on (design.md
 * 9.16: one line of consequence stays visible). The rest of the facts go behind the info icon when
 * they fit one, and under Details otherwise, each on its own line and word for word.
 */
function HeadedFindingLine({ prompt }: { prompt: string }) {
  const [summary = '', ...facts] = findingText(prompt)
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
  const [firstFact = '', ...otherFacts] = facts;
  const { lead: factLead, tip, details } = foldForHelp(firstFact);
  const rest = [tip, details, ...otherFacts].filter(Boolean);
  const restText = rest.join(' ');
  const inlineTip = rest.length > 0 && fitsInfoTip(restText) ? restText : '';
  return (
    <div data-testid="tool-security-finding" className="flex min-w-0 flex-col gap-1">
      <div className="flex items-start gap-2 text-body text-text-default">
        <AlertTriangle aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-text-warning" />
        <span data-testid="tool-security-finding-summary" className="min-w-0">
          {summary}
        </span>
      </div>
      {factLead ? (
        <p className="min-w-0 pl-6 text-body text-text-default [overflow-wrap:anywhere]">
          {factLead}
          {inlineTip ? (
            <>
              {' '}
              <InfoTip label={APPROVAL_COPY.findingName} help={inlineTip} />
            </>
          ) : null}
        </p>
      ) : null}
      {rest.length > 0 && !inlineTip && (
        <div className="pl-6">
          <Disclosure label={APPROVAL_COPY.findingDetails}>
            <div className="flex flex-col gap-2 text-body text-text-default">
              {rest.map((part, index) => (
                <p key={index} className="min-w-0 [overflow-wrap:anywhere]">
                  {part}
                </p>
              ))}
            </div>
          </Disclosure>
        </div>
      )}
    </div>
  );
}

/**
 * A refusal or an error, as one line with the rest behind an info icon or a Details disclosure.
 * `help`, when given, is the info icon's text for a line the app wrote itself; otherwise the line
 * is the daemon's sentence, folded after its first sentence when it runs past one line. A line
 * that fits stays whole: the first line of a refusal or an error is never hidden.
 */
function FoldedNotice({
  text,
  help,
  name,
  role,
  className,
}: {
  text: string;
  help?: string;
  name: string;
  role: 'status' | 'alert';
  className: string;
}) {
  const { lead, tip, details } = help ? { lead: text, tip: help, details: '' } : foldForHelp(text);
  return (
    <div role={role} className={className}>
      <p className="min-w-0 [overflow-wrap:anywhere]">
        {lead}
        {tip ? (
          <>
            {' '}
            <InfoTip label={name} help={tip} />
          </>
        ) : null}
      </p>
      {details ? (
        <Disclosure label={APPROVAL_COPY.findingDetails}>
          <p className="min-w-0 [overflow-wrap:anywhere]">{details}</p>
        </Disclosure>
      ) : null}
    </div>
  );
}

/**
 * A decision on the folder-access card, with what it does on hover (the app's label tooltip) and
 * as its accessible description, so the consequence is in reach at the moment of the press
 * without a paragraph above the buttons.
 */
function DecisionButton({
  help,
  children,
  ...props
}: ComponentProps<typeof Button> & { help: string }) {
  const helpId = useId();
  return (
    <>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button {...props} aria-describedby={helpId}>
            {children}
          </Button>
        </TooltipTrigger>
        <TooltipContent>{help}</TooltipContent>
      </Tooltip>
      <span id={helpId} className="sr-only">
        {help}
      </span>
    </>
  );
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
          typeof explanation === 'string' ? explanation : APPROVAL_COPY.folder.refused
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

  // The finding is a security fact, so its first line stays visible on every
  // state of the card: a warning line, not a tinted band (a band inside a card
  // is a box in a box). Only the triangle wears the warning hue; the sentence is
  // the primary ink, because a paragraph in amber reads as an alarm and reads
  // worse (owner, 2026-11-12: permission prompts ask for attention without
  // alarm; design.md 9.14). The rest folds (`FindingLine`). A Crew card's prompt is a summary
  // line, then a line per fact, and folds by its lines (`HeadedFindingLine`).
  const findingLine = securityFinding ? (
    crewPerson ? (
      <HeadedFindingLine prompt={prompt} />
    ) : (
      <FindingLine prompt={prompt} />
    )
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
          {/* A folder card's prompt restates its preview; the answered row says enough. */}
          {findingLine && !folderAccess && <div className="pl-6">{findingLine}</div>}
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
        {/* A folder-access card's prompt is the daemon's long restatement of its own
            preview, so it is not drawn: the question, its info icon and the folded file
            list carry it (owner, 2026-11-13: one line up front, the detail on hover). */}
        {!folderAccess && findingLine}

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
              <span className="inline-flex items-center gap-1">
                <span data-testid="folder-access-question">
                  {FOLDER_ACCESS_COPY.question({
                    requester: folderRequester,
                    access: preview.access,
                    folder: folderShown(preview.folder, preview.files),
                  })}
                </span>
                {/* What a grant covers and how long it lasts, on hover (owner, 2026-11-13: the
                    question is enough up front; the disclaimer is the small icon beside it). The
                    exact paths and terms stay under Details. */}
                <InfoTip label="this folder access" help={folderAccessHelp(preview)} />
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
            <span className="inline-flex items-center gap-1">
              <label htmlFor={`${toolConfirmationId}-folder-key`}>Folder approval key</label>
              <InfoTip label="the folder approval key" help={FOLDER_KEY_HELP} />
            </span>
            <input
              id={`${toolConfirmationId}-folder-key`}
              type="password"
              autoComplete="off"
              value={folderApprovalKey}
              onChange={(event) => setFolderApprovalKey(event.target.value)}
              className="rounded border border-border-default bg-background-default px-2 py-1"
              aria-describedby={`${toolConfirmationId}-folder-key-help`}
            />
            {/* Read by a screen reader with the field; seen on hover through the InfoTip. */}
            <span id={`${toolConfirmationId}-folder-key-help`} className="sr-only">
              {FOLDER_KEY_HELP}
            </span>
          </div>
        )}
        {cannotApprove ? (
          // A refusal, so its line stays visible (principle 2); why it cannot be
          // answered here is the info icon beside it.
          <FoldedNotice
            role="status"
            className="text-body text-text-muted"
            text={cannotApprove}
            help={
              cannotApprove === BROWSER_CANNOT_APPROVE
                ? APPROVAL_COPY.browserCannotApproveWhy
                : undefined
            }
            name={APPROVAL_COPY.browserCannotApproveName}
          />
        ) : folderAccess && preview?.kind === 'folderAccess' ? (
          // Each decision says what it does on hover, so the card needs no
          // paragraph above the buttons (owner, 2026-11-13).
          <div className="flex flex-wrap items-center justify-end gap-2">
            <DecisionButton
              type="button"
              size="sm"
              variant="default"
              disabled={isSending || (browserFolderAccess && !folderApprovalKey)}
              onClick={() => handleButtonClick(ALLOW_ONCE)}
              help={APPROVAL_COPY.folder.allowOnceHelp(Boolean(preview.onceDescendants))}
            >
              {APPROVAL_COPY.allowOnce}
            </DecisionButton>
            <DecisionButton
              type="button"
              size="sm"
              variant="secondary"
              disabled={isSending || (browserFolderAccess && !folderApprovalKey)}
              onClick={() => handleButtonClick(ALLOW_SESSION)}
              help={APPROVAL_COPY.folder.allowSessionHelp}
            >
              {FOLDER_ACCESS_COPY.allowSession}
            </DecisionButton>
            <DecisionButton
              type="button"
              size="sm"
              variant="ghost"
              disabled={isSending}
              onClick={() => handleButtonClick(DENY)}
              help={APPROVAL_COPY.folder.denyHelp}
            >
              {APPROVAL_COPY.deny}
            </DecisionButton>
            <DecisionButton
              type="button"
              size="sm"
              variant="ghost"
              disabled={isSending}
              onClick={() => handleButtonClick(CANCEL)}
              help={APPROVAL_COPY.folder.cancelHelp}
            >
              {FOLDER_ACCESS_COPY.cancel}
            </DecisionButton>
          </div>
        ) : (
          <div className="flex flex-wrap items-center justify-end gap-2">
            <Button
              type="button"
              size="sm"
              variant="default"
              disabled={isSending}
              onClick={() => handleButtonClick(ALLOW_ONCE)}
            >
              {readConsent ? APPROVAL_COPY.readConsent.allow : APPROVAL_COPY.allowOnce}
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
          // An error that just happened keeps its first line visible; the
          // folder card's own fallback says what to check behind its info icon.
          <FoldedNotice
            role="alert"
            className="text-supporting text-text-danger"
            text={confirmationError}
            help={
              confirmationError === APPROVAL_COPY.folder.refused
                ? APPROVAL_COPY.folder.refusedHelp
                : undefined
            }
            name={APPROVAL_COPY.folder.refusedName}
          />
        )}
      </div>

      {/* Modal for updating tool permission */}
      {isModalOpen && (
        <PermissionModal onClose={handleModalClose} extensionName={getExtensionName(toolName)} />
      )}
    </>
  );
}
