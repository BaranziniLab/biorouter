/**
 * ExtensionUpdateReporter.tsx
 *
 * Surfaces the background extension updater's results.
 *
 * The updater has been broadcasting `extension-update-event` on a startup timer
 * for some time and **nothing in the renderer listened**. An extension that
 * failed to update — usually on `uv sync`, i.e. a Python dependency that will not
 * build on this machine — failed silently and stayed on its old version, with the
 * only trace a line in a log file the user never opens.
 *
 * Renders nothing. It exists to turn those events into a toast, and to give a
 * failed update the same "Debug with Biorouter" way out that a failed install has.
 */
import { useEffect } from 'react';
import { toastError, toastService } from '../toasts';

export default function ExtensionUpdateReporter() {
  useEffect(() => {
    // Optional-chained deliberately. This sits in the app shell, so a host that
    // does not provide the channel — headless/browser mode, a partially stubbed
    // `window.electron` in a test — would otherwise take the whole app down over
    // a notification nobody asked for.
    const dispose = window.electron?.onExtensionUpdateEvent?.((event) => {
      if (event.type === 'update-error') {
        toastError({
          title: `Could not update ${event.displayName || event.ext || 'an extension'}`,
          msg: event.error || 'The update failed.',
          debugFailure: {
            kind: 'extension',
            name: event.ext || 'extension',
            displayName: event.displayName,
            // The updater re-runs `uv sync` after unpacking, and that is where
            // this fails in practice.
            command: 'uv sync',
          },
        });
        return;
      }

      // Successes are worth exactly one line, and only when something changed:
      // "0 extensions updated" on every launch is noise.
      if (event.type === 'all-done' && (event.updatedCount ?? 0) > 0) {
        const n = event.updatedCount ?? 0;
        toastService.success({
          title: 'Extensions updated',
          msg: `${n} extension${n === 1 ? '' : 's'} updated to the latest version.`,
        });
      }
    });

    // Unsubscribe on unmount. This is NOT belt-and-braces: the comment that
    // used to sit here claimed the reporter was "mounted once, at the app
    // shell, for the life of the window", and that was false twice over. It
    // renders inside `AppLayout`, which sits under `ProviderGuard >
    // ChatProvider`, so the subtree remounts; and React.StrictMode is on in
    // dev, so even a single mount runs this effect twice (effect → cleanup →
    // effect). Every one of those mounts used to add an IPC listener that was
    // never removed, which is the `MaxListenersExceededWarning: 11
    // extension-update-event listeners` measured in #189 — every surviving
    // listener also fires its own duplicate toast.
    //
    // `dispose` is optional-chained for the same reason the subscribe call is:
    // a host without the channel returns nothing to call.
    return () => dispose?.();
  }, []);

  return null;
}
