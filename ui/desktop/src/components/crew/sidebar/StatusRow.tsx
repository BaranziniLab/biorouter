import type { ReactElement } from 'react';
import { LoaderCircle } from '../../icons/app-icons';
import { StatusDot } from '../../ui/status-dot';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { useCrew } from '../state/CrewControllerContext';
import { CONNECTION_STATUS, type ConnectionStatusKey } from '../state/crewStatus';
import { sidebarCopy } from './copy';
import { PrivacyChip, privacyCheckDeferred } from './PrivacyChip';
import { usePendingHost, useSidebarView } from './sidebarView';
import './crew-sidebar.css';

/**
 * What a status word waits for, or `null` when the word says it all (Q2-17, Q2-43). A tooltip
 * that repeats its own label tells nobody anything, so only these two carry one: "Updates
 * unavailable" says where the fix is, and "Not joined yet" says whose turn it is.
 */
export function statusHint(
  status: ConnectionStatusKey,
  workspace: string,
  host: string | null
): string | null {
  if (status === 'updates-unavailable') return sidebarCopy.statusHint.updatesUnavailable(workspace);
  if (status === 'not-joined') return sidebarCopy.statusHint.notJoined(host);
  return null;
}

/**
 * The status row (ui-redesign-spec, "The Crew sidebar" and "Connection status"): the one resting
 * home of security state, 32px under the workspace name, in every state once a connection is
 * selected.
 *
 * Left, a `role="status"` region: the dot and the word the controller derived. When healthy the
 * word reads "Connected" and a visually hidden sentence carries the pinned **Connected · identity
 * verified**, so a screen reader hears the verified sentence once and never the short word too.
 * "Sign-in needed" is a button that opens Sign in; "Connecting…" carries the one spinner.
 *
 * Right, the privacy chip — which says nothing while nothing is verifying privacy (offline,
 * reconnecting, a join the host has not let in yet…), so the row never reads "Offline · Checking
 * privacy…" (Q2-17), and nothing while the word itself says a check runs ("Checking connection",
 * "Connecting…"), so the row reads its status whole instead of "Checking connection · Checking
 * pri…" (Q3-55). Then the status region still says both facts to a screen reader: "Checking
 * connection. Checking privacy…".
 *
 * A word with more to say carries it in a tooltip BELOW the row, never over the workspace name
 * above it, and in the region itself for a screen reader: "Updates unavailable" names the
 * workspace and where Retry is, and "Not joined yet" says who has to let you in (Q2-43). No
 * tooltip repeats the word it sits on. Nothing here animates a change of state: security state
 * swaps instantly, so no frame shows a stale status mid-tween.
 */
export function StatusRow() {
  const crew = useCrew();
  const { status, openSignIn } = crew;
  const { title } = useSidebarView(crew);
  const { host } = usePendingHost(crew);
  if (!crew.connection || !status) return null;
  const presentation = CONNECTION_STATUS[status];
  const hint = statusHint(status, title, host);
  const tooltip = hint ?? presentation.srText ?? null;
  // What the chip would have said, while the word stands in for it (Q3-55).
  const privacyPending = privacyCheckDeferred(crew);
  const spoken = [hint, privacyPending ? sidebarCopy.chip.checking : null].filter(Boolean);

  const word =
    status === 'sign-in-needed' ? (
      <button type="button" className="crew-sidebar-status-button no-drag" onClick={openSignIn}>
        {presentation.word}
      </button>
    ) : presentation.srText ? (
      <>
        <WordTooltip text={tooltip}>
          <span className="crew-sidebar-truncate" aria-hidden="true" data-crew-status-word="">
            {presentation.word}
          </span>
        </WordTooltip>
        <span className="sr-only">{presentation.srText}</span>
      </>
    ) : (
      <>
        <WordTooltip text={tooltip}>
          <span className="crew-sidebar-truncate" data-crew-status-word="">
            {presentation.word}
          </span>
        </WordTooltip>
        {spoken.length > 0 && (
          <span className="sr-only" data-crew-status-more="">{`. ${spoken.join('. ')}`}</span>
        )}
      </>
    );

  return (
    <div className="crew-sidebar-status" data-crew-status={status}>
      <div role="status" aria-label={sidebarCopy.statusLabel} className="crew-sidebar-status-word">
        {presentation.spinner ? (
          <LoaderCircle className="crew-sidebar-spinner" aria-hidden="true" />
        ) : (
          <StatusDot tone={presentation.tone} />
        )}
        {word}
      </div>
      <PrivacyChip />
    </div>
  );
}

/**
 * The word's tooltip, opening BELOW the row and start-aligned, so it covers the sidebar's first
 * rows for a moment rather than the workspace name above (Q2-43: it covered the switcher).
 */
function WordTooltip({ text, children }: { text: string | null; children: ReactElement }) {
  if (!text) return children;
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent side="bottom" align="start" data-crew-status-tooltip="">
        {text}
      </TooltipContent>
    </Tooltip>
  );
}
