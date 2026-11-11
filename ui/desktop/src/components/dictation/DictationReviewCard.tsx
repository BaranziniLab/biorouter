import { useEffect, useRef, useState } from 'react';
import { PageHeader } from '../Layout/PageHeader';
import { ReadableContent } from '../Layout/ReadableContent';
import { Button } from '../ui/button';
import { Note } from '../ui/note';
import { Textarea } from '../ui/textarea';
import { dictationMessage } from './copy';

export default function DictationReviewCard() {
  const [text, setText] = useState('');
  const [message, setMessage] = useState('');
  const [confirmPrivate, setConfirmPrivate] = useState(false);
  const lastRevision = useRef(-1);
  const expiryTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    let mounted = true;
    const receive = (next: {
      text?: string;
      message?: string;
      confirmPrivate?: boolean;
      expiresAt: number | null;
      revision: number;
    }) => {
      if (!mounted || next.revision < lastRevision.current) return;
      lastRevision.current = next.revision;
      if (expiryTimer.current) clearTimeout(expiryTimer.current);
      const live = next.expiresAt !== null && Date.now() < next.expiresAt;
      setText(live ? (next.text ?? '') : '');
      setMessage(next.message ?? '');
      setConfirmPrivate(next.confirmPrivate === true);
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
  return (
    <main className="min-h-screen bg-background-canvas" aria-label="Dictation review">
      <PageHeader title="Dictation" />
      <ReadableContent size="chat" className="space-y-4 px-4 py-4">
        {message && (
          <Note tone="neutral" role="status">
            {message}
          </Note>
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
              onClick={() => void window.electron.dictationAnywhereReviewCopy?.()}
            >
              Copy
            </Button>
          </div>
        )}
      </ReadableContent>
    </main>
  );
}
