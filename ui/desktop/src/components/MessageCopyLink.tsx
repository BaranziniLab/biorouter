import React from 'react';
import { AlertTriangle, Copy } from './icons/app-icons';
import { MessageMetaAction } from './MessageMeta';
import { useTransientValue } from '../hooks/useTransientFlag';
import { toastError } from '../toasts';

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

  const handleCopy = async () => {
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

    // Fallback to plain text if the rich copy failed.
    try {
      await navigator.clipboard.writeText(text);
      markOutcome('copied');
      return;
    } catch (fallbackErr) {
      console.error('Failed to copy text (fallback): ', fallbackErr);
    }

    // ⚠ Both writes failed, and this is the branch that used to end at a
    // `console.error` the user cannot see: `markCopied()` was never reached, so
    // the button went on saying "Copy" and the only way to find out nothing had
    // been copied was to paste. Say so, in both places the user might be
    // looking — on the control they pressed, and once in the corner.
    markOutcome('failed');
    toastError({
      title: 'Copy failed',
      msg: 'Biorouter could not write to the clipboard. Select the message and copy it with your keyboard.',
    });
  };

  const failed = outcome === 'failed';

  return (
    <MessageMetaAction
      onClick={handleCopy}
      icon={failed ? <AlertTriangle /> : <Copy />}
      aria-label="Copy message"
      className={failed ? 'text-text-warning hover:text-text-warning' : undefined}
    >
      {outcome === 'copied' ? 'Copied!' : failed ? 'Copy failed' : 'Copy'}
    </MessageMetaAction>
  );
}
