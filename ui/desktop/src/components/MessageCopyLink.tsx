import React from 'react';
import { Copy } from './icons/app-icons';
import { MessageMetaAction } from './MessageMeta';
import { useTransientFlag } from '../hooks/useTransientFlag';

interface MessageCopyLinkProps {
  text: string;
  contentRef: React.RefObject<HTMLDivElement | null>;
}

export default function MessageCopyLink({ text, contentRef }: MessageCopyLinkProps) {
  const [copied, markCopied] = useTransientFlag(2000);

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

      markCopied();
    } catch (err) {
      console.error('Failed to copy text: ', err);
      // Fallback to plain text if HTML copy fails
      try {
        await navigator.clipboard.writeText(text);
        markCopied();
      } catch (fallbackErr) {
        console.error('Failed to copy text (fallback): ', fallbackErr);
      }
    }
  };

  return (
    <MessageMetaAction onClick={handleCopy} icon={<Copy />} aria-label="Copy message">
      {copied ? 'Copied!' : 'Copy'}
    </MessageMetaAction>
  );
}
