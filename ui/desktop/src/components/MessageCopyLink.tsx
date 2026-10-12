import React from 'react';
import { AlertTriangle, Check, Copy } from './icons/app-icons';
import { TranscriptIconButton } from './transcript/TranscriptIconButton';
import { transcriptCopy } from './transcript/copy';
import { useTransientValue } from '../hooks/useTransientFlag';
import { toastError } from '../toasts';
import { copyToClipboard } from '../utils/clipboard';

interface MessageCopyLinkProps {
  text: string;
  contentRef: React.RefObject<HTMLDivElement | null>;
}

/** What the clipboard write is asked to put on the clipboard. */
type CopyOutcome = 'copied' | 'failed';

export default function MessageCopyLink({ text, contentRef }: MessageCopyLinkProps) {
  // One transient state with two values rather than two booleans that could
  // both be true: the button says exactly one thing at a time.
  const [outcome, markOutcome] = useTransientValue<CopyOutcome>(2000);

  const handleCopy = async (event: React.MouseEvent<HTMLButtonElement>) => {
    // Read before the first await: React clears `currentTarget` once dispatch ends.
    // The message is the host for the document-selection fallback; a cluster
    // that is still fading in is no worse a host than its parent was.
    const host =
      event.currentTarget.closest<HTMLElement>('.br-message-actions-scope') ??
      event.currentTarget.parentElement;
    try {
      if (contentRef?.current) {
        // Clone the DOM node to avoid innerHTML re-serialization
        const container = contentRef.current.cloneNode(true) as HTMLDivElement;

        // Clean up any copy buttons from the content
        const copyButtons = container.querySelectorAll('button');
        copyButtons.forEach((button) => button.remove());

        // Create the clipboard data
        const clipboardData = new ClipboardItem({
          'text/plain': new Blob([text], { type: 'text/plain' }),
          'text/html': new Blob([container.innerHTML], { type: 'text/html' }),
        });

        await navigator.clipboard.write([clipboardData]);
      } else {
        await navigator.clipboard.writeText(text);
      }

      markOutcome('copied');
      return;
    } catch (err) {
      console.error('Failed to copy text: ', err);
    }

    // Fallback to plain text if the rich copy failed, through the shared path
    // (utils/clipboard.ts): retried after focusing the window, then copied
    // through the document's own selection, which no permission handler gates.
    if (await copyToClipboard(text, host)) {
      markOutcome('copied');
      return;
    }

    // ⚠ Both writes failed, and this is the branch that used to end at a
    // `console.error` the user cannot see: `markCopied()` was never reached, so
    // the button went on saying "Copy" and the only way to find out nothing had
    // been copied was to paste. Say so, in both places the user might be
    // looking — on the control they pressed, and once in the corner.
    markOutcome('failed');
    toastError({
      title: transcriptCopy.message.copyFailed,
      msg: transcriptCopy.message.copyFailedHelp,
    });
  };

  const failed = outcome === 'failed';
  const label =
    outcome === 'copied'
      ? transcriptCopy.message.copied
      : failed
        ? transcriptCopy.message.copyFailed
        : transcriptCopy.message.copyMessage;

  // A glyph button that answers in its tooltip ("Copied"), so its width never
  // jumps (Crew's `CopyIconButton`). The name follows the state.
  return (
    <TranscriptIconButton
      onClick={handleCopy}
      label={label}
      tip={outcome === null ? transcriptCopy.message.copy : label}
      holdTip={outcome !== null}
      tone={failed ? 'warning' : 'default'}
      icon={
        outcome === 'copied' ? (
          <Check aria-hidden="true" />
        ) : failed ? (
          <AlertTriangle aria-hidden="true" />
        ) : (
          <Copy aria-hidden="true" />
        )
      }
    />
  );
}
