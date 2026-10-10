import { useLayoutEffect, useRef, type ClipboardEvent, type KeyboardEvent } from 'react';

import { SESSION_NAME_MAX_LENGTH } from '../../utils/sessionNameSync';
import { chatRowCopy } from './copy';

export interface ChatRowRenameInputProps {
  /** The name the row showed, for the input's accessible name and its starting text. */
  title: string;
  /** The draft, held by the list so a head refresh that re-renders the row keeps it. */
  value: string;
  onChange: (value: string) => void;
  /** Enter, Tab or blur. The caller normalises and decides whether anything changed. */
  onCommit: (value: string) => void;
  /** Escape. */
  onCancel: () => void;
  className?: string;
}

/**
 * The in-place editor for a chat row's name (spec 3.4, owner message 4): the
 * sidebar uses it today, and the tab strip can reuse it for its Rename.
 *
 * - Opens with all the text selected.
 * - Enter, Tab and blur commit; Escape cancels. Escape is default-prevented and
 *   stopped, because the overlay sidebar's window-level Escape closes the panel
 *   unless an Escape was already answered (`ui/sidebar.tsx`).
 * - Enter while an IME is composing is the IME's, not a commit.
 * - Pasted newlines become spaces (an `<input>` would drop them and glue the
 *   words together).
 * - 200 characters at most, the server's cap.
 *
 * The commit runs once: a blur that follows an Enter or Escape is ignored.
 */
export function ChatRowRenameInput({
  title,
  value,
  onChange,
  onCommit,
  onCancel,
  className,
}: ChatRowRenameInputProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const settled = useRef(false);

  useLayoutEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    input.focus();
    input.select();
  }, []);

  const commit = () => {
    if (settled.current) return;
    settled.current = true;
    onCommit(inputRef.current?.value ?? value);
  };

  const cancel = () => {
    if (settled.current) return;
    settled.current = true;
    onCancel();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    event.stopPropagation();
    if (event.key === 'Enter') {
      if (event.nativeEvent.isComposing) return;
      event.preventDefault();
      commit();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      cancel();
    } else if (event.key === 'Tab') {
      event.preventDefault();
      commit();
    }
  };

  const onPaste = (event: ClipboardEvent<HTMLInputElement>) => {
    const pasted = event.clipboardData.getData('text');
    if (!/[\r\n]/.test(pasted)) return;
    event.preventDefault();
    const input = event.currentTarget;
    const text = pasted.replace(/[\r\n]+/g, ' ');
    const start = input.selectionStart ?? input.value.length;
    const end = input.selectionEnd ?? input.value.length;
    const next = (input.value.slice(0, start) + text + input.value.slice(end)).slice(
      0,
      SESSION_NAME_MAX_LENGTH
    );
    onChange(next);
    const caret = Math.min(start + text.length, next.length);
    requestAnimationFrame(() => input.setSelectionRange(caret, caret));
  };

  return (
    <input
      ref={inputRef}
      type="text"
      className={className ?? 'br-chat-row-rename-input'}
      value={value}
      maxLength={SESSION_NAME_MAX_LENGTH}
      aria-label={chatRowCopy.rename.inputLabel(title)}
      spellCheck={false}
      autoComplete="off"
      onChange={(event) => onChange(event.target.value)}
      onKeyDown={onKeyDown}
      onPaste={onPaste}
      onBlur={commit}
    />
  );
}
