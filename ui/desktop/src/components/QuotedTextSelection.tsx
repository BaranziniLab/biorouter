import {
  previewSelectionFromMessage,
  hasSelectedText,
  type SelectedText,
} from '../utils/previewTextSelection';
import { useEffect, useState, type RefObject, type ReactElement } from 'react';
import {
  ContextMenu,
  ContextMenuTrigger,
  ContextMenuContent,
  ContextMenuItem,
} from './ui/context-menu';
import { sendQuotedText, type QuoteSource } from '../utils/quotedText';
import { toastError } from '../toasts';

export function textSelectedInside(root: HTMLElement | null): string {
  const selection = root?.ownerDocument.getSelection();
  if (
    !root ||
    !selection ||
    selection.isCollapsed ||
    !selection.anchorNode ||
    !selection.focusNode ||
    !root.contains(selection.anchorNode) ||
    !root.contains(selection.focusNode)
  )
    return '';
  return selection.toString();
}

export function useTextSelection(root: RefObject<HTMLElement | null>, sourceKey: string) {
  const [selection, setSelection] = useState<{ key: string; text: SelectedText }>({
    key: sourceKey,
    text: '',
  });
  useEffect(() => {
    const ownerDocument = root.current?.ownerDocument ?? document;
    const update = () => {
      const text = textSelectedInside(root.current);
      setSelection((previous) =>
        previous.key === sourceKey && previous.text === text ? previous : { key: sourceKey, text }
      );
    };
    const receive = (event: MessageEvent) => {
      if (!root.current) return;
      const text = previewSelectionFromMessage(root.current, event);
      if (text !== null) setSelection({ key: sourceKey, text });
    };
    ownerDocument.addEventListener('selectionchange', update);
    ownerDocument.defaultView?.addEventListener('message', receive);
    return () => {
      ownerDocument.removeEventListener('selectionchange', update);
      ownerDocument.defaultView?.removeEventListener('message', receive);
    };
  }, [root, sourceKey]);
  return selection.key === sourceKey ? selection.text : '';
}

export function attachSelectedText(source: QuoteSource, text: SelectedText, origin?: HTMLElement) {
  try {
    if (typeof text !== 'string') throw new Error(text.error);
    sendQuotedText({ source, text }, origin);
  } catch (error) {
    toastError({
      title: 'Could not attach selected text',
      msg: error instanceof Error ? error.message : 'Please select the text again.',
    });
  }
}

export function QuotedTextSelection({
  source,
  selection,
  children,
}: {
  source: QuoteSource;
  selection: SelectedText;
  children: ReactElement;
}) {
  const [snapshot, setSnapshot] = useState<{
    source: QuoteSource;
    text: SelectedText;
    origin: HTMLElement;
  } | null>(null);
  return (
    <ContextMenu>
      <ContextMenuTrigger
        asChild
        disabled={!hasSelectedText(selection)}
        onContextMenu={(event) => {
          setSnapshot({ source: { ...source }, text: selection, origin: event.currentTarget });
        }}
      >
        {children}
      </ContextMenuTrigger>
      <ContextMenuContent onCloseAutoFocus={(event) => event.preventDefault()}>
        <ContextMenuItem
          onSelect={() =>
            snapshot && attachSelectedText(snapshot.source, snapshot.text, snapshot.origin)
          }
        >
          Ask about it
        </ContextMenuItem>
        <ContextMenuItem
          onSelect={() =>
            snapshot && attachSelectedText(snapshot.source, snapshot.text, snapshot.origin)
          }
        >
          Quote it
        </ContextMenuItem>
        <ContextMenuItem
          disabled={!!snapshot && typeof snapshot.text !== 'string'}
          onSelect={() => {
            if (!snapshot || typeof snapshot.text !== 'string') return;
            const copy = window.electron?.copySelectedText
              ? window.electron.copySelectedText(snapshot.text)
              : navigator.clipboard.writeText(snapshot.text);
            void copy.catch(() =>
              toastError({ title: 'Could not copy text', msg: 'Please try copying again.' })
            );
          }}
        >
          Copy
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}
