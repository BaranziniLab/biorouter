import { useEffect, useState } from 'react';
import { Note } from '../ui/note';
import { Button } from '../ui/button';
import { IconAction } from '../ui/row-actions';
import { AlertCircle, X } from '../icons/app-icons';
import { extensionDisplayName } from '../settings/extensions/extensionDisplayName';
import {
  dismissExtensionLoadFailure,
  getExtensionLoadFailures,
  subscribeExtensionLoadFailures,
  type ExtensionLoadFailure,
} from '../../utils/extensionLoadFailures';
import { createExtensionRecoverHints } from '../../utils/extensionErrorUtils';
import { useTransientValue } from '../../hooks/useTransientFlag';
import { LOAD_FAILURE_COPY } from './copy';
import './extension-notice.css';

/**
 * A button label that changes in place without moving anything beside it (V3-R4-5).
 *
 * "Copy error" turns into "Copied" for two seconds, and the button is as wide as its label, so
 * the neighbour on its left (Ask Biorouter) slid 22px right and back. Both labels sit in one
 * grid cell instead: the one on show is visible, the other is `visibility: hidden` and
 * `aria-hidden`, so the cell, and with it the button, is as wide as the longer of the two
 * whatever the font, text size or language. A fixed `min-width` would be right for one of those
 * and wrong for the rest. The rule is authored in `extension-notice.css`.
 */
function StableLabel({ shown, other }: { shown: string; other: string }) {
  return (
    <span className="br-ext-notice-swap">
      <span>{shown}</span>
      <span aria-hidden="true">{other}</span>
    </span>
  );
}

/**
 * The standing extension-failure notice, on the page the failure is ABOUT.
 *
 * The defect this closes had two halves and they compounded: a toast recurred
 * on every renderer load (fixed in `extensionLoadFailures`), and the Extensions
 * page it sent you to answered "No extensions yet" — the destination denied
 * that the thing that had just failed existed at all. An extension that fails
 * at load can be absent from the configured list for perfectly ordinary reasons
 * (it was removed while a chat still referenced it, it is session-scoped, its
 * config row failed to parse), and in every one of them the page owes the user
 * the name, the reason and a next step rather than an empty state.
 *
 * Tier: this is a Banner, not a Toast (`toasts.tsx`) — a standing condition
 * that must stay put, not a confirmation that expires. It carries per-item
 * actions, which is also why it is not a toast.
 */
export function ExtensionLoadFailureNotice({
  onAskBiorouter,
}: {
  /** Hands the failure to the agent. Omitted where no chat can be started. */
  onAskBiorouter?: (hints: string) => void;
}) {
  const [failures, setFailures] = useState<ExtensionLoadFailure[]>(() =>
    getExtensionLoadFailures()
  );
  const [copied, markCopied] = useTransientValue<string>(2000);

  useEffect(() => {
    // Re-read on mount as well as subscribing: the record is written by the
    // chat shell's `/agent/resume`, which has usually already run by the time
    // anyone navigates here, so a subscription alone would show nothing.
    setFailures(getExtensionLoadFailures());
    return subscribeExtensionLoadFailures(() => setFailures(getExtensionLoadFailures()));
  }, []);

  if (failures.length === 0) return null;

  return (
    <div className="mb-4 flex flex-col gap-2" data-testid="extension-load-failures">
      {failures.map((failure) => (
        <Note
          key={failure.name}
          tone="danger"
          role="alert"
          icon={AlertCircle}
          action={
            // Two quiet text actions and a round X, in the Note's one trailing slot: the notice
            // is a line and its error, not a card with a second row of buttons under it.
            <div className="br-ext-notice-actions">
              {onAskBiorouter && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => onAskBiorouter(createExtensionRecoverHints(failure.error))}
                >
                  {LOAD_FAILURE_COPY.askBiorouter}
                </Button>
              )}
              <Button
                size="sm"
                variant="ghost"
                onClick={() => {
                  navigator.clipboard?.writeText(failure.error);
                  markCopied(failure.name);
                }}
              >
                <StableLabel
                  shown={
                    copied === failure.name ? LOAD_FAILURE_COPY.copied : LOAD_FAILURE_COPY.copyError
                  }
                  other={
                    copied === failure.name ? LOAD_FAILURE_COPY.copyError : LOAD_FAILURE_COPY.copied
                  }
                />
              </Button>
              <IconAction
                icon={X}
                label={LOAD_FAILURE_COPY.dismiss}
                size="sm"
                onSelect={() => dismissExtensionLoadFailure(failure.name)}
              />
            </div>
          }
        >
          {/* The extension's name as the catalogue spells it (CDWAgent), never its config key
              title-cased (Cdwagent): principle 10, and the name the Extensions row uses. */}
          <div className="text-label text-text-default">
            {LOAD_FAILURE_COPY.failedToLoad(extensionDisplayName(failure.name))}
          </div>
          {/* The error, which the Note clamps behind "Show more" past its ceiling. This
              surface has room, and the toast's hard truncation is what sent people looking for
              details in the first place. */}
          <div className="mt-1 [overflow-wrap:anywhere]">{failure.error}</div>
        </Note>
      ))}
    </div>
  );
}

export default ExtensionLoadFailureNotice;
