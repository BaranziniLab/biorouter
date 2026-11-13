import { useEffect, useRef, useState } from 'react';
import { PageHeader } from '../Layout/PageHeader';
import { ReadableContent } from '../Layout/ReadableContent';
import { Button } from '../ui/button';
import { Note } from '../ui/note';
import { Textarea } from '../ui/textarea';
import { DICTATION_ACTIONS, dictationMessage } from './copy';

/** The copy shortcut for MSG-CLIPBOARD-COPY-FAILED, as main names the paste one. */
function copyKey(): string {
  return window.electron?.platform === 'darwin' ? '⌘C' : 'Ctrl+C';
}

export default function DictationReviewCard() {
  const [text, setText] = useState('');
  const [message, setMessage] = useState('');
  const [confirmPrivate, setConfirmPrivate] = useState(false);
  const [retry, setRetry] = useState(false);
  // Copy reads the clipboard back in main (`dictation:anywhere:review-copy`), as the first copy
  // did; a copy the clipboard did not keep says so here rather than looking done.
  const [copyFailed, setCopyFailed] = useState(false);
  const lastRevision = useRef(-1);
  const expiryTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    let mounted = true;
    const receive = (next: {
      text?: string;
      message?: string;
      confirmPrivate?: boolean;
      retry?: boolean;
      expiresAt: number | null;
      revision: number;
    }) => {
      if (!mounted || next.revision < lastRevision.current) return;
      const changed = next.revision !== lastRevision.current;
      lastRevision.current = next.revision;
      if (expiryTimer.current) clearTimeout(expiryTimer.current);
      const live = next.expiresAt !== null && Date.now() < next.expiresAt;
      setText(live ? (next.text ?? '') : '');
      setMessage(next.message ?? '');
      setConfirmPrivate(next.confirmPrivate === true);
      setRetry(next.retry === true);
      if (changed) setCopyFailed(false);
      if (live && next.text) {
        const revision = next.revision;
        expiryTimer.current = setTimeout(
          () => {
            if (lastRevision.current === revision) setText('');
            expiryTimer.current = null;
          },
          Math.max(0, next.expiresAt! - Date.now())
        );
      }
    };
    const remove = window.electron.onDictationAnywhereReview?.(receive);
    void window.electron.dictationAnywhereReviewGet?.().then((next) => {
      if (next) receive(next);
    });
    return () => {
      mounted = false;
      remove?.();
      if (expiryTimer.current) clearTimeout(expiryTimer.current);
    };
  }, []);
  const copy = async () => {
    setCopyFailed(false);
    let copied = false;
    try {
      copied = (await window.electron.dictationAnywhereReviewCopy?.()) === true;
    } catch {
      copied = false;
    }
    setCopyFailed(!copied);
  };
  return (
    <main className="min-h-screen bg-background-canvas" aria-label="Dictation review">
      <PageHeader title="Dictation" />
      <ReadableContent size="chat" className="space-y-4 px-4 py-4">
        {message && (
          <Note tone="neutral" role="status">
            {message}
          </Note>
        )}
        {retry && (
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              variant="default"
              onClick={() => void window.electron.dictationAnywhereReviewRetry?.()}
            >
              {DICTATION_ACTIONS.retry}
            </Button>
          </div>
        )}
        {confirmPrivate && (
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              variant="secondary"
              onClick={() => void window.electron.dictationAnywherePrivateChoice?.(true)}
            >
              Include
            </Button>
            <Button
              type="button"
              variant="default"
              onClick={() => void window.electron.dictationAnywherePrivateChoice?.(false)}
            >
              Public terms only
            </Button>
          </div>
        )}
        {text && (
          <div className="space-y-4">
            <Note tone="info">{dictationMessage('MSG-CLIPBOARD-EXPLAIN')}</Note>
            <Textarea aria-label="Dictated text" readOnly value={text} />
            <Button
              type="button"
              variant={confirmPrivate ? 'secondary' : 'default'}
              onClick={() => void copy()}
            >
              Copy
            </Button>
            {copyFailed && (
              <Note tone="neutral" role="status">
                {dictationMessage('MSG-CLIPBOARD-COPY-FAILED', { key: copyKey() })}
              </Note>
            )}
          </div>
        )}
      </ReadableContent>
    </main>
  );
}
