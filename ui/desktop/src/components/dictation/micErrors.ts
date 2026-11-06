/**
 * Why the microphone did not open, as a sentence and at most one action
 * ("Error map" in `docs/dictation/surfaces.md`).
 *
 * The answer depends on the `getUserMedia` error and, on the desktop, on what
 * the operating system says about Biorouter's microphone access
 * (`dictation:mic-status`). Errors are shown in the composer card's notice row,
 * never as toasts, and the row clears when the next dictation starts.
 */
import type { MicAccessStatus } from '../../utils/micPermissionSettings';
import { dictationMessage, type DictationMessageId } from './copy';

export type MicErrorAction = 'open-settings' | null;

export interface MicErrorNotice {
  id: DictationMessageId;
  text: string;
  action: MicErrorAction;
}

export interface MicErrorContext {
  /** `process.platform` of the app (the desktop) or of the browser's computer, as best known. */
  platform: string;
  /** `utils/surface.ts` `currentSurface()`. */
  surface: 'desktop' | 'browser';
  /** The operating system's answer, on the desktop; `unknown` elsewhere. */
  osStatus: MicAccessStatus;
  /** For MSG-INSECURE: the page's host. */
  host?: string;
}

const notice = (
  id: DictationMessageId,
  action: MicErrorAction = null,
  values: Record<string, string> = {}
): MicErrorNotice => ({ id, text: dictationMessage(id, values), action });

const nameOf = (error: unknown): string =>
  error && typeof error === 'object' && 'name' in error ? String(error.name) : '';

/**
 * The notice for a capture that failed to start. `OverconstrainedError` reaches
 * here only after `capture.ts` retried by label and with the default device, so
 * it means there is no usable device.
 */
export function micErrorNotice(error: unknown, context: MicErrorContext): MicErrorNotice {
  const name = nameOf(error);
  if (name === 'InsecureContextError') {
    return notice('MSG-INSECURE', null, context.host ? { host: context.host } : {});
  }
  switch (name) {
    case 'NotAllowedError':
    case 'PermissionDeniedError':
    case 'SecurityError':
      return notAllowed(context);
    case 'NotFoundError':
    case 'DevicesNotFoundError':
    case 'OverconstrainedError':
      return notice('MSG-NO-DEVICE');
    case 'NotReadableError':
    case 'TrackStartError':
    case 'AbortError':
      return notice('MSG-MIC-BUSY');
    default:
      return notice('MSG-MIC-BUSY');
  }
}

function notAllowed(context: MicErrorContext): MicErrorNotice {
  if (context.surface === 'browser') return notice('MSG-MIC-BROWSER');
  if (context.platform === 'win32') {
    // Which status Windows reports with the switch off is not verified (S3.2),
    // so every denial on Windows names the switch and opens its page.
    return notice('MSG-MIC-DENIED-WIN', 'open-settings');
  }
  if (context.platform === 'darwin') {
    switch (context.osStatus) {
      case 'restricted':
        return notice('MSG-MIC-RESTRICTED');
      case 'granted':
        return notice('MSG-MIC-GRANTED-FAILED');
      default:
        return notice('MSG-MIC-DENIED-MAC', 'open-settings');
    }
  }
  // Linux: no operating system switch, so a refusal is a busy or blocked device.
  return notice('MSG-MIC-BUSY');
}
