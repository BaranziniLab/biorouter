import { useState } from 'react';
import { Loader2 } from './icons/app-icons';
import { Button } from './ui/button';
import { ModalShell } from './ModalShell';
import { NotificationContent, type NotificationStatus } from './alerts/NotificationSurface';
import { startNewSession } from '../sessions';
import { useNavigation } from '../hooks/useNavigation';
import { formatExtensionErrorMessage } from '../utils/extensionErrorUtils';
import { getInitialWorkingDir } from '../utils/workingDir';
import { formatExtensionName } from './settings/extensions/subcomponents/ExtensionList';
import { useTransientValue } from '../hooks/useTransientFlag';

export interface ExtensionLoadingStatus {
  name: string;
  status: 'loading' | 'success' | 'error';
  error?: string;
  recoverHints?: string;
}

interface ExtensionLoadingToastProps {
  /**
   * The extensions the RATIO is over — the user's own, i.e. exactly the
   * population the composer's extension menu lists. Anything that ships with
   * Biorouter is excluded upstream so the two surfaces can be read against each
   * other; see `showExtensionLoadResults`.
   */
  extensions: ExtensionLoadingStatus[];
  totalCount: number;
  isComplete: boolean;
  /**
   * Names of built-in capabilities that failed to load. Reported by name and
   * NEVER folded into the ratio: they are real news (a broken capability
   * resurfaces later as a broken tool call) but they are not things the user
   * installed, so counting them makes the denominator disagree with the menu.
   */
  builtinFailures?: string[];
}

// Per-row markers: 8px status dots (design.md §4.16), tokenised for both themes.
function StatusDot({ status }: { status: ExtensionLoadingStatus['status'] }) {
  if (status === 'loading') {
    return <Loader2 className="w-3.5 h-3.5 animate-spin text-text-info" />;
  }
  return (
    <span
      className={`block w-2 h-2 rounded-full ${
        status === 'success' ? 'bg-background-success' : 'bg-background-danger'
      }`}
    />
  );
}

/**
 * The extension-load report: everything the toast is not allowed to carry.
 *
 * §3.7's tier model draws the line at "anything with a list, a disclosure, or
 * per-item actions" — this panel is all three (one row per extension, each with
 * its own pair of buttons), and it used to live *inside* a 400px-tall
 * collapsible toast in the transient layer, where a 5s timer could take the
 * failure report away mid-read.
 */
function ExtensionLoadReport({
  open,
  onOpenChange,
  extensions,
  onAskBiorouter,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  extensions: ExtensionLoadingStatus[];
  onAskBiorouter: ((hints: string) => void) | null;
}) {
  // The 2s "Copied!" label. `useTransientValue` owns the timer, so dismissing
  // the report inside the window cannot leave a setter pointed at a dead tree.
  const [copiedExtension, markCopied] = useTransientValue<string>(2000);
  const errorCount = extensions.filter((ext) => ext.status === 'error').length;

  return (
    <ModalShell
      open={open}
      onOpenChange={onOpenChange}
      // A report with a list: width L.
      size="lg"
      purpose="info"
      scrollBody
      title="Extension load report"
      subtitle={`${errorCount} of ${extensions.length} extension${
        extensions.length !== 1 ? 's' : ''
      } failed to load.`}
      footer={
        <Button variant="outline" size="sm" onClick={() => onOpenChange(false)}>
          Close
        </Button>
      }
    >
      <div className="space-y-3 py-3">
        {extensions.map((ext) => (
          <div key={ext.name} className="flex items-start gap-2.5 text-[13px]">
            <span className="flex-none mt-1">
              <StatusDot status={ext.status} />
            </span>
            <div className="flex-1 min-w-0">
              <div className="truncate text-text-default">{formatExtensionName(ext.name)}</div>
              {ext.status === 'error' && ext.error && (
                <>
                  <div className="mt-1 text-supporting text-text-muted [overflow-wrap:anywhere]">
                    {formatExtensionErrorMessage(ext.error, 'Failed to add extension')}
                  </div>
                  <div className="mt-1.5 flex flex-wrap gap-2">
                    {ext.recoverHints && onAskBiorouter && (
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => onAskBiorouter(ext.recoverHints!)}
                      >
                        Ask Biorouter
                      </Button>
                    )}
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => {
                        navigator.clipboard.writeText(ext.error!);
                        markCopied(ext.name);
                      }}
                    >
                      {copiedExtension === ext.name ? 'Copied!' : 'Copy error'}
                    </Button>
                  </div>
                </>
              )}
            </div>
          </div>
        ))}
      </div>
    </ModalShell>
  );
}

/**
 * The grouped extension-load toast, held to the toast tier's ceiling: a status
 * chip, a title, at most three wrapped lines, and at most two quiet actions.
 * The per-extension detail is one click away in {@link ExtensionLoadReport}
 * rather than nested inside the toast.
 */
export function GroupedExtensionLoadingToast({
  extensions,
  isComplete,
  builtinFailures = [],
}: ExtensionLoadingToastProps) {
  const [reportOpen, setReportOpen] = useState(false);
  const setView = useNavigation();

  const errorCount = extensions.filter((ext) => ext.status === 'error').length;
  const failedNames = extensions
    .filter((ext) => ext.status === 'error')
    .map((ext) => formatExtensionName(ext.name));
  const builtinFailureNames = builtinFailures.map(formatExtensionName);

  // Summary line. On success we deliberately say "All extensions loaded" rather
  // than a count (the all-success case has no meaningful denominator). On
  // PARTIAL failure a bare "N failed" hides how much still works, so we lead
  // with what landed — "3 of 4 extensions loaded" — and name the casualties
  // underneath.
  //
  // ⚠ The ratio is over the USER's extensions only, which is the same set the
  // composer's extension menu lists, so the two numbers can be read against
  // each other. A built-in capability that failed gets its own sentence rather
  // than a slot in the denominator — see the props above for why.
  const summary = !isComplete
    ? 'Loading extensions…'
    : extensions.length === 0
      ? // Nothing of the user's own was involved, so there is no ratio to give:
        // the built-in failure IS the whole message.
        `${builtinFailureNames.length} built-in ${
          builtinFailureNames.length === 1 ? 'capability' : 'capabilities'
        } failed to load`
      : errorCount === 0 && builtinFailureNames.length === 0
        ? 'All extensions loaded'
        : errorCount === 0
          ? // Every extension the user added is fine; only a built-in broke. Say
            // so, rather than an "All extensions loaded" title under an error chip.
            'All extensions loaded, but a built-in capability failed'
          : `${extensions.length - errorCount} of ${extensions.length} extension${
              extensions.length !== 1 ? 's' : ''
            } loaded`;

  const status: NotificationStatus = !isComplete
    ? 'loading'
    : errorCount === 0 && builtinFailureNames.length === 0
      ? 'success'
      : 'error';

  return (
    <>
      <NotificationContent
        status={status}
        title={summary}
        message={
          [
            failedNames.length > 0 ? `Failed: ${failedNames.join(', ')}` : null,
            // Marked as built-in so the extra name cannot be mistaken for an
            // extension missing from the menu.
            builtinFailureNames.length > 0 && extensions.length > 0
              ? `Built-in: ${builtinFailureNames.join(', ')}`
              : null,
          ]
            .filter(Boolean)
            .join(' · ') || undefined
        }
        clampMessage
        actions={
          errorCount > 0 ? (
            <Button
              size="sm"
              variant="secondary"
              onClick={(e) => {
                // The toast is the click target for dismissal; opening the
                // report must not also close the thing that reports it.
                e.stopPropagation();
                setReportOpen(true);
              }}
            >
              View details
            </Button>
          ) : undefined
        }
      />
      {errorCount > 0 && (
        <ExtensionLoadReport
          open={reportOpen}
          onOpenChange={setReportOpen}
          extensions={extensions}
          onAskBiorouter={
            setView ? (hints) => startNewSession(getInitialWorkingDir(), hints, setView) : null
          }
        />
      )}
    </>
  );
}
