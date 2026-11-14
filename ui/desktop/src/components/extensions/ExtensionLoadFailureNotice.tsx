import { useEffect, useState } from 'react';
import { Note } from '../ui/note';
import { Button } from '../ui/button';
import { IconAction } from '../ui/row-actions';
import { AlertCircle, X } from '../icons/app-icons';
import { extensionDisplayName } from '../settings/extensions/extensionDisplayName';
import {
  dismissExtensionLoadFailure,
  getExtensionLoadFailures,
  showingExtensionLoadFailureNotice,
  subscribeExtensionLoadFailures,
  type ExtensionLoadFailure,
} from '../../utils/extensionLoadFailures';
import {
  createExtensionRecoverHints,
  withdrawExtensionFailureToasts,
} from '../../utils/extensionErrorUtils';
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

/** The fold under an error's first line that holds the rest of its output. */
const ERROR_DETAILS_LABEL = 'Details';

/** An error's first non-empty line, and everything after it (trimmed), for the fold. */
function errorLines(error: string): { first: string; rest: string } {
  const text = error.trim();
  const newline = text.indexOf('\n');
  if (newline < 0) return { first: text, rest: '' };
  return { first: text.slice(0, newline).trim(), rest: text.slice(newline + 1).trim() };
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

  // While this page is open it is where a load failure is announced, including one recorded while
  // it is open, and the corner's report of the same failure is taken down rather than left over
  // the page's controls: once this notice was dismissed, the sticky "1 of 2 extensions loaded"
  // covered the list's first switch (final check, 2026-10-12). See `extensionLoadFailures.ts`.
  useEffect(() => showingExtensionLoadFailureNotice(), []);
  useEffect(() => {
    if (failures.length > 0) withdrawExtensionFailureToasts();
  }, [failures]);

  if (failures.length === 0) return null;

  return (
    <div className="mb-4 flex flex-col gap-2" data-testid="extension-load-failures">
      {failures.map((failure) => {
        const error = errorLines(failure.error);
        return (
          <Note
            key={failure.name}
            tone="danger"
            role="alert"
            icon={AlertCircle}
            // The rest of a multi-line error goes in the Note's own `details`, which it renders
            // outside its clamped node. Folded inside `children`, opening it grew the content past
            // the Note's ceiling and the stack opened behind a fade, a second "Show more" away.
            details={
              error.rest ? (
                <pre className="whitespace-pre-wrap font-mono text-code [overflow-wrap:anywhere]">
                  {error.rest}
                </pre>
              ) : undefined
            }
            detailsLabel={ERROR_DETAILS_LABEL}
          >
            {/* A line and its error, with the actions in a column at the trailing edge: two quiet
              text actions and a round X, centred on the title, with no second row of buttons
              under the error, which wraps before them. The Note's own action slot did this, but
              its slot cannot see the title beside it, and the actions now step aside: below about
              1600 wide the toast layer's corner reaches over the reading column's right end, and
              the sticky extension-load report there covered Copy error and the X (final smoke).
              While a toast stands beside them, `data-toast-aside` moves them left, out from under
              it, as far as the title allows (`alerts/toastClearance.ts`); the error and
              everything below stay put. */}
            <div className="br-ext-notice-body">
              {/* The extension's name as the catalogue spells it (CDWAgent), never its config key
                title-cased (Cdwagent): principle 10, and the name the Extensions row uses. */}
              <div className="br-ext-notice-name text-label text-text-default">
                {LOAD_FAILURE_COPY.failedToLoad(extensionDisplayName(failure.name))}
              </div>
              <div className="br-ext-notice-actions" data-toast-aside="">
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
                      copied === failure.name
                        ? LOAD_FAILURE_COPY.copied
                        : LOAD_FAILURE_COPY.copyError
                    }
                    other={
                      copied === failure.name
                        ? LOAD_FAILURE_COPY.copyError
                        : LOAD_FAILURE_COPY.copied
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
              {/* The error's first line, always: it just happened, and it is what a person reads
                first. The rest of a multi-line error (a process's output, a stack) folds under
                "Details" (the Note's `details`, above) in the code face, since it is output to
                read character by character (owner, 2026-11-13: one line up front, the detail a
                click away). Copy error still copies the whole thing. */}
              <div className="br-ext-notice-error mt-1 [overflow-wrap:anywhere]">
                <div>{error.first}</div>
              </div>
            </div>
          </Note>
        );
      })}
    </div>
  );
}

export default ExtensionLoadFailureNotice;
