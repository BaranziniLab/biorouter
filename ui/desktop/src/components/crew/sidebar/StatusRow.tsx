import { LoaderCircle } from '../../icons/app-icons';
import { StatusDot } from '../../ui/status-dot';
import { useCrew } from '../state/CrewControllerContext';
import { CONNECTION_STATUS } from '../state/crewStatus';
import { sidebarCopy } from './copy';
import { PrivacyChip } from './PrivacyChip';
import './crew-sidebar.css';

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
 * Right, the privacy chip. Nothing here animates a change of state: security state swaps
 * instantly, so no frame shows a stale status mid-tween.
 */
export function StatusRow() {
  const crew = useCrew();
  const { status, openSignIn } = crew;
  if (!crew.connection || !status) return null;
  const presentation = CONNECTION_STATUS[status];

  return (
    <div className="crew-sidebar-status" data-crew-status={status}>
      <div role="status" aria-label={sidebarCopy.statusLabel} className="crew-sidebar-status-word">
        {presentation.spinner ? (
          <LoaderCircle className="crew-sidebar-spinner" aria-hidden="true" />
        ) : (
          <StatusDot tone={presentation.tone} />
        )}
        {status === 'sign-in-needed' ? (
          <button type="button" className="crew-sidebar-status-button no-drag" onClick={openSignIn}>
            {presentation.word}
          </button>
        ) : presentation.srText ? (
          <>
            <span aria-hidden="true">{presentation.word}</span>
            <span className="sr-only">{presentation.srText}</span>
          </>
        ) : (
          <span className="crew-sidebar-truncate">{presentation.word}</span>
        )}
      </div>
      <PrivacyChip />
    </div>
  );
}
