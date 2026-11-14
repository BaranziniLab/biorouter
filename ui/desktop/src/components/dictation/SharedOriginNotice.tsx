import { useId } from 'react';
import { ModalShell } from '../ModalShell';
import { Button } from '../ui/button';
import { DICTATION_ACTIONS, dictationMessage } from './copy';

/** The acknowledgement click itself starts capture while it has user activation. */
export function SharedOriginNotice({
  open,
  onCancel,
  onContinue,
}: {
  open: boolean;
  onCancel: () => void;
  onContinue: () => void;
}) {
  const messageId = useId();
  const origin = typeof window === 'undefined' ? '' : window.location.origin;
  return (
    <ModalShell
      open={open}
      onOpenChange={(next) => {
        if (!next) onCancel();
      }}
      size="sm"
      purpose="info"
      title="Microphone permission"
      describedBy={messageId}
      footer={
        <>
          <Button type="button" variant="secondary" onClick={onCancel}>
            {DICTATION_ACTIONS.cancel}
          </Button>
          <Button type="button" data-testid="shared-origin-acknowledge" onClick={onContinue}>
            Continue
          </Button>
        </>
      }
    >
      <p id={messageId} className="text-body text-text-muted">
        {dictationMessage('MSG-SHARED-ORIGIN', { origin })}
      </p>
    </ModalShell>
  );
}
