import { useState } from 'react';
import { ActionRequired } from '../api';
import JsonSchemaForm from './ui/JsonSchemaForm';
import type { JsonSchema } from './ui/JsonSchemaForm';
import { Check, X } from './icons/app-icons';
import { errorMessage } from '../utils/conversionUtils';
import { TranscriptRow } from './TranscriptRow';
import { ELICITATION_COPY } from './toolCallCopy';

interface ElicitationRequestProps {
  isCancelledMessage: boolean;
  isClicked: boolean;
  actionRequiredContent: ActionRequired & { type: 'actionRequired' };
  onSubmit: (elicitationId: string, userData: Record<string, unknown>) => Promise<void>;
}

export default function ElicitationRequest({
  isCancelledMessage,
  isClicked,
  actionRequiredContent,
  onSubmit,
}: ElicitationRequestProps) {
  const [submitted, setSubmitted] = useState(isClicked);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (actionRequiredContent.data.actionType !== 'elicitation') {
    return null;
  }

  const { id: elicitationId, message, requested_schema } = actionRequiredContent.data;

  const handleSubmit = async (formData: Record<string, unknown>) => {
    if (submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      await onSubmit(elicitationId, formData);
      setSubmitted(true);
    } catch (failure) {
      setError(errorMessage(failure));
    } finally {
      setSubmitting(false);
    }
  };

  // Resolved states are transcript rows: the card has done its job.
  if (isCancelledMessage) {
    return (
      <div className="biorouter-message-content text-body">
        <TranscriptRow icon={X} label={ELICITATION_COPY.canceled} />
      </div>
    );
  }

  if (submitted) {
    return (
      <div className="biorouter-message-content text-body">
        <TranscriptRow icon={Check} label={ELICITATION_COPY.sent} />
      </div>
    );
  }

  // Asking: the one card recipe. The question is the card's title; the form
  // and its action follow. One box, not a bubble glued to a form.
  return (
    <div className="biorouter-message-content br-enter text-body flex min-w-0 flex-col gap-3 rounded-container border border-border-subtle bg-background-default p-4">
      <div className="text-label text-text-default">
        {message || ELICITATION_COPY.fallbackQuestion}
      </div>
      {error && (
        <p role="alert" className="text-supporting text-text-danger">
          {error}
        </p>
      )}
      <fieldset disabled={submitting} className="min-w-0">
        <JsonSchemaForm
          schema={requested_schema as JsonSchema}
          onSubmit={handleSubmit}
          submitLabel={submitting ? ELICITATION_COPY.submitting : ELICITATION_COPY.submit}
        />
      </fieldset>
    </div>
  );
}
