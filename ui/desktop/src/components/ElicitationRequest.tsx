import { useState } from 'react';
import { ActionRequired } from '../api';
import JsonSchemaForm from './ui/JsonSchemaForm';
import type { JsonSchema } from './ui/JsonSchemaForm';
import { Check, X } from './icons/app-icons';
import { errorMessage } from '../utils/conversionUtils';
import { TranscriptRow } from './TranscriptRow';
import { ELICITATION_COPY } from './toolCallCopy';
import { Disclosure } from './ui/disclosure';
import { InfoTip } from './ui/info-tip';
import { foldForHelp } from '../utils/proseFold';

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
  //
  // A server often writes a paragraph of instructions here. The title is its
  // first sentence; the rest is the info icon beside it, or a Details
  // disclosure under it when longer, so the form stays in reach (owner,
  // 2026-11-13: no solid chunk of text, every word one hover or click away).
  const { lead, tip, details } = foldForHelp(message || ELICITATION_COPY.fallbackQuestion);
  return (
    <div className="biorouter-message-content br-action-card br-fill-surface br-enter text-body">
      <div className="text-label text-text-default">
        {lead}
        {tip ? (
          <>
            {' '}
            <InfoTip label={ELICITATION_COPY.moreName} help={tip} />
          </>
        ) : null}
      </div>
      {details ? (
        <Disclosure label={ELICITATION_COPY.more}>
          <p className="whitespace-pre-wrap text-body text-text-default">{details}</p>
        </Disclosure>
      ) : null}
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
