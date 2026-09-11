import { toast, ToastOptions } from 'react-toastify';
import { Button } from './components/ui/button';
import { NotificationContent } from './components/alerts/NotificationSurface';
import { startNewSession } from './sessions';
import { useNavigation } from './hooks/useNavigation';
import {
  GroupedExtensionLoadingToast,
  ExtensionLoadingStatus,
} from './components/GroupedExtensionLoadingToast';
import { getInitialWorkingDir } from './utils/workingDir';
import { startChatFailureNotice } from './utils/startChatFailure';
import { launchDependencyDebugSession } from './utils/launchDependencyDebug';
import type { DependencyFailure } from './utils/dependencyDebugPrompt';

export interface ToastServiceOptions {
  silent?: boolean;
  shouldThrow?: boolean;
}

/**
 * One id for the grouped extension toast, deliberately shared across chats: with
 * tabs and splits, four chats can finish loading at once, and four separate
 * toasts reporting the same broken extension would bury the transcript. One id
 * coalesces them into a single, updating report.
 */
const EXTENSION_TOAST_ID = 'extension-loading';

class ToastService {
  private silent: boolean = false;
  private shouldThrow: boolean = false;

  // Create a singleton instance
  private static instance: ToastService;

  public static getInstance(): ToastService {
    if (!ToastService.instance) {
      ToastService.instance = new ToastService();
    }
    return ToastService.instance;
  }

  configure(options: ToastServiceOptions = {}): void {
    if (options.silent !== undefined) {
      this.silent = options.silent;
    }

    if (options.shouldThrow !== undefined) {
      this.shouldThrow = options.shouldThrow;
    }
  }

  error(props: ToastErrorProps): void {
    if (!this.silent) {
      toastError(props);
    }

    if (this.shouldThrow) {
      throw new Error(props.msg);
    }
  }

  loading({ title, msg }: { title: string; msg: string }): string | number | undefined {
    if (this.silent) {
      return undefined;
    }

    const toastId = toastLoading({ title, msg });

    return toastId;
  }

  /**
   * `toastOptions` overrides the shared defaults for this one toast. The case it
   * exists for is a message that must not be able to expire unread — see the
   * BR-71 chatrecall suggestion, which is shown exactly once per install and so
   * passes `{ autoClose: false }`. Everything else should keep the 3s default.
   */
  success({ title, msg }: { title: string; msg: string }, toastOptions: ToastOptions = {}): void {
    if (this.silent) {
      return;
    }
    toastSuccess({ title, msg, toastOptions });
  }

  dismiss(toastId?: string | number): void {
    if (toastId) toast.dismiss(toastId);
  }

  /**
   * Whether the grouped extension toast is still on screen. Lets a caller avoid
   * overwriting a live failure report with a later success — see
   * `showExtensionLoadResults`.
   */
  isExtensionToastActive(): boolean {
    if (this.silent) {
      return false;
    }
    return toast.isActive(EXTENSION_TOAST_ID);
  }

  /**
   * Create a grouped extension loading toast that can be updated as extensions load
   */
  extensionLoading(
    extensions: ExtensionLoadingStatus[],
    totalCount: number,
    isComplete: boolean = false,
    // Built-in capabilities that failed. Carried separately from `extensions`
    // so the toast's ratio stays over the same population the composer's
    // extension menu shows; see `showExtensionLoadResults`.
    builtinFailures: string[] = []
  ): string | number {
    if (this.silent) {
      return 'silent';
    }

    const toastId = EXTENSION_TOAST_ID;

    // Drive react-toastify's own icon + theme (same as toastSuccess/toastError,
    // i.e. the model-change toast) so this toast looks consistent: a spinner
    // while loading, a green check when all loaded, the error theme on failure.
    const hasErrors =
      extensions.some((ext) => ext.status === 'error') || builtinFailures.length > 0;
    const type: 'success' | 'error' | 'default' = !isComplete
      ? 'default'
      : hasErrors
        ? 'error'
        : 'success';

    if (toast.isActive(toastId)) {
      // Update existing toast
      toast.update(toastId, {
        render: (
          <GroupedExtensionLoadingToast
            extensions={extensions}
            totalCount={totalCount}
            isComplete={isComplete}
            builtinFailures={builtinFailures}
          />
        ),
        isLoading: !isComplete,
        type,
        icon: false,
        // A clean run expires like any other confirmation; a FAILURE report
        // persists, because its "View details" action opens a dialog and a 5s
        // timer must not be able to take the toast (and with it the dialog)
        // away mid-read.
        autoClose: isComplete && !hasErrors ? TOAST_AUTO_CLOSE_MS : false,
        closeButton: true,
        closeOnClick: false,
      });
    } else {
      // Create new toast
      toast(
        <GroupedExtensionLoadingToast
          extensions={extensions}
          totalCount={totalCount}
          isComplete={isComplete}
          builtinFailures={builtinFailures}
        />,
        {
          ...commonToastOptions,
          toastId,
          isLoading: !isComplete,
          type,
          icon: false,
          autoClose: isComplete && !hasErrors ? TOAST_AUTO_CLOSE_MS : false,
          closeButton: true,
          closeOnClick: false, // the toast carries an action; see the tier rules
        }
      );
    }

    return toastId;
  }

  /**
   * Handle errors with consistent logging and toast notifications
   * Consolidates the functionality of the original handleError function
   */
  handleError(title: string, message: string, options: ToastServiceOptions = {}): void {
    this.configure(options);
    this.error({
      title: title,
      msg: message,
      traceback: message,
    });
  }
}

// Export a singleton instance for use throughout the app
export const toastService = ToastService.getInstance();

// Re-export ExtensionLoadingStatus for convenience
export type { ExtensionLoadingStatus };

/**
 * THE TOAST TIER (Astryx §3.7). Three tiers exist and the boundary is enforced
 * here rather than negotiated per call site:
 *
 *   Toast   — title + at most three wrapped lines + at most two quiet actions.
 *             Everything in this file.
 *   Banner  — an in-place notice that must stay put; `NotificationSurface`.
 *   Report  — anything with a list, a disclosure, or per-item actions. It is a
 *             dialog, not a toast; see `GroupedExtensionLoadingToast`, whose
 *             400px collapsible panel is what this tier exists to evict from
 *             the transient layer.
 *
 * The three rules the tier carries, each of which the app used to break:
 *   · ONE dwell time. 5s for everything that expires; errors do not expire, so
 *     a failure cannot vanish before it is read.
 *   · A toast that carries actions is NOT click-to-dismiss — an error toast
 *     used to die if you clicked 2px off its own button.
 *   · DEDUP BY KEY IS POLICY, not the single exception the extension toast used
 *     to be. Identical notifications coalesce instead of stacking N deep.
 */
export const TOAST_AUTO_CLOSE_MS = 5000;

const commonToastOptions: ToastOptions = {
  position: 'top-right',
  closeButton: true,
  hideProgressBar: true,
  closeOnClick: true,
  pauseOnHover: true,
  draggable: true,
  // ⚠ **Off, and this is what makes `autoClose` mean anything.**
  // react-toastify defaults `pauseOnFocusLoss` to true, so every dismissal
  // timer stops the moment the window is not frontmost — and a notification
  // the user has already walked away from is exactly the one that most needs
  // to expire. The observed result was a stack of toasts still sitting there
  // minutes later, which reads as "these need dismissing" rather than "these
  // were FYI". `pauseOnHover` stays on: that one pauses because the user is
  // reading it, which is a reason to wait.
  pauseOnFocusLoss: false,
};

/**
 * The dedup key for a content-identical toast. Deliberately NOT applied to
 * `toastLoading`: a loading toast's id is a handle its caller holds to dismiss
 * exactly its own operation, and collapsing two concurrent operations onto one
 * id would let the first to finish dismiss the second's progress.
 */
const dedupeKey = (kind: string, title?: string, msg?: string) =>
  `${kind}:${title ?? ''}:${msg ?? ''}`;

type ToastSuccessProps = { title?: string; msg?: string; toastOptions?: ToastOptions };

export function toastSuccess({ title, msg, toastOptions = {} }: ToastSuccessProps) {
  return toast.success(
    // title AND msg render independently — a message with no title is no longer dropped.
    <NotificationContent status="success" title={title} message={msg} clampMessage />,
    {
      ...commonToastOptions,
      icon: false,
      autoClose: TOAST_AUTO_CLOSE_MS,
      toastId: dedupeKey('success', title, msg),
      ...toastOptions,
    }
  );
}

export function toastInfo({ title, msg, toastOptions = {} }: ToastSuccessProps) {
  return toast.info(
    <NotificationContent status="info" title={title} message={msg} clampMessage />,
    {
      ...commonToastOptions,
      icon: false,
      autoClose: TOAST_AUTO_CLOSE_MS,
      toastId: dedupeKey('info', title, msg),
      ...toastOptions,
    }
  );
}

export function toastWarning({ title, msg, toastOptions = {} }: ToastSuccessProps) {
  return toast.warning(
    <NotificationContent status="warning" title={title} message={msg} clampMessage />,
    {
      ...commonToastOptions,
      icon: false,
      autoClose: TOAST_AUTO_CLOSE_MS,
      toastId: dedupeKey('warning', title, msg),
      ...toastOptions,
    }
  );
}

type ToastErrorProps = {
  title: string;
  msg: string;
  traceback?: string;
  recoverHints?: string;
  /**
   * An install or prerequisite failure. Adds "Debug with Biorouter", which opens
   * a NEW window briefed with the failure and this machine's environment.
   *
   * Distinct from `recoverHints` on purpose: that one hands a hint to the CURRENT
   * view via `startNewSession`, which is right for an extension that failed to
   * load in the chat you are looking at. A failed install is usually background
   * work interrupting something else, so it gets its own window instead of
   * replacing what the user was doing.
   */
  debugFailure?: Omit<DependencyFailure, 'environment' | 'error'>;
};

function ToastErrorContent({
  title,
  msg,
  traceback,
  recoverHints,
  debugFailure,
}: Omit<ToastErrorProps, 'setView'>) {
  const setView = useNavigation();
  const showRecovery = recoverHints && setView;

  const handleCopyError = async () => {
    if (traceback) {
      try {
        await navigator.clipboard.writeText(traceback);
      } catch (error) {
        console.error('Failed to copy error:', error);
      }
    }
  };

  // Actions sit BELOW the message in the shared primitive's action row, so they
  // can never crowd or slide under the close ×.
  const actions =
    showRecovery || traceback || debugFailure ? (
      <>
        {debugFailure && (
          <Button
            size="sm"
            variant="secondary"
            onClick={() => void launchDependencyDebugSession({ ...debugFailure, error: msg })}
          >
            Debug with Biorouter
          </Button>
        )}
        {showRecovery && (
          <Button
            size="sm"
            variant="secondary"
            onClick={() =>
              void startNewSession(getInitialWorkingDir(), recoverHints, setView).catch((error) =>
                toastError(startChatFailureNotice(error, { kept: false }))
              )
            }
          >
            Ask Biorouter
          </Button>
        )}
        {traceback && (
          <Button size="sm" variant="secondary" onClick={handleCopyError}>
            Copy error
          </Button>
        )}
      </>
    ) : undefined;

  return (
    <NotificationContent
      status="error"
      title={title}
      message={msg}
      clampMessage
      actions={actions}
    />
  );
}

export function toastError({ title, msg, traceback, recoverHints, debugFailure }: ToastErrorProps) {
  // An error toast carries actions whenever there is something to copy or a
  // recovery path to offer — and a toast with actions is not click-to-dismiss,
  // because the click that misses the button must not destroy the button.
  const hasActions = Boolean(traceback || recoverHints || debugFailure);
  return toast.error(
    <ToastErrorContent
      title={title}
      msg={msg}
      traceback={traceback}
      recoverHints={recoverHints}
      debugFailure={debugFailure}
    />,
    {
      ...commonToastOptions,
      icon: false,
      // Errors persist. A failure that expires unread is a failure that was
      // never reported.
      autoClose: false,
      closeOnClick: !hasActions,
      toastId: dedupeKey('error', title, msg),
    }
  );
}

type ToastLoadingProps = {
  title?: string;
  msg?: string;
  toastOptions?: ToastOptions;
};

export function toastLoading({ title, msg, toastOptions }: ToastLoadingProps) {
  return toast.loading(
    <NotificationContent status="loading" title={title} message={msg} clampMessage />,
    {
      ...commonToastOptions,
      icon: false,
      autoClose: false,
      ...toastOptions,
    }
  );
}
