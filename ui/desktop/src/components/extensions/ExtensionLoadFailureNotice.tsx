import { useEffect, useState } from 'react';
import { Note } from '../ui/note';
import { Button } from '../ui/button';
import { AlertCircle } from '../icons/app-icons';
import { formatExtensionName } from '../settings/extensions/subcomponents/ExtensionList';
import {
  dismissExtensionLoadFailure,
  getExtensionLoadFailures,
  subscribeExtensionLoadFailures,
  type ExtensionLoadFailure,
} from '../../utils/extensionLoadFailures';
import { createExtensionRecoverHints } from '../../utils/extensionErrorUtils';
import { useTransientValue } from '../../hooks/useTransientFlag';

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
    <div className="mb-6 space-y-2" data-testid="extension-load-failures">
      {failures.map((failure) => (
        <Note
          key={failure.name}
          tone="danger"
          role="alert"
          icon={AlertCircle}
          action={
            <Button
              size="sm"
              variant="ghost"
              onClick={() => dismissExtensionLoadFailure(failure.name)}
            >
              Dismiss
            </Button>
          }
        >
          <div className="font-medium text-text-default">
            {formatExtensionName(failure.name)} failed to load
          </div>
          {/* The FULL error, not the toast's truncation. This surface has room,
              and the truncated form is what sent people looking for details in
              the first place. */}
          <div className="mt-1 [overflow-wrap:anywhere]">{failure.error}</div>
          <div className="mt-2 flex flex-wrap gap-2">
            {onAskBiorouter && (
              <Button
                size="sm"
                variant="secondary"
                onClick={() => onAskBiorouter(createExtensionRecoverHints(failure.error))}
              >
                Ask Biorouter
              </Button>
            )}
            <Button
              size="sm"
              variant="secondary"
              onClick={() => {
                navigator.clipboard?.writeText(failure.error);
                markCopied(failure.name);
              }}
            >
              {copied === failure.name ? 'Copied!' : 'Copy error'}
            </Button>
          </div>
        </Note>
      ))}
    </div>
  );
}

export default ExtensionLoadFailureNotice;
