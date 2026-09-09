import { useState } from 'react';
import { ActionRequired } from '../api';
import JsonSchemaForm from './ui/JsonSchemaForm';
import type { JsonSchema } from './ui/JsonSchemaForm';
import { Check } from './icons/app-icons';

interface ElicitationRequestProps {
  isCancelledMessage: boolean;
  isClicked: boolean;
  actionRequiredContent: ActionRequired & { type: 'actionRequired' };
  onSubmit: (elicitationId: string, userData: Record<string, unknown>) => void;
}

export default function ElicitationRequest({
  isCancelledMessage,
  isClicked,
  actionRequiredContent,
  onSubmit,
}: ElicitationRequestProps) {
  const [submitted, setSubmitted] = useState(isClicked);

  if (actionRequiredContent.data.actionType !== 'elicitation') {
    return null;
  }

  const { id: elicitationId, message, requested_schema } = actionRequiredContent.data;

  const handleSubmit = (formData: Record<string, unknown>) => {
    setSubmitted(true);
    onSubmit(elicitationId, formData);
  };

  if (isCancelledMessage) {
    return (
      <div className="biorouter-message-content bg-background-muted rounded-2xl px-4 py-2 text-body text-text-default">
        Information request was canceled.
      </div>
    );
  }

  if (submitted) {
    return (
      <div className="biorouter-message-content bg-background-muted rounded-2xl px-4 py-2 text-body text-text-default">
        <div className="flex items-center gap-2">
          <Check className="w-5 h-5 text-text-muted" />
          <span>Information submitted</span>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col">
      <div className="biorouter-message-content bg-background-muted rounded-2xl rounded-b-none px-4 py-2 text-body text-text-default">
        {message || 'Biorouter needs some information from you.'}
      </div>
      <div className="biorouter-message-content bg-background-default border border-border-subtle rounded-b-2xl px-4 py-3 text-body">
        <JsonSchemaForm
          schema={requested_schema as JsonSchema}
          onSubmit={handleSubmit}
          submitLabel="Submit"
        />
      </div>
    </div>
  );
}
