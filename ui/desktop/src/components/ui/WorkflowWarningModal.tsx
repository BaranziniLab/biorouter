import * as React from 'react';
import { ModalShell } from '../ModalShell';
import { Button } from './button';
import { Note } from './note';
import MarkdownContent from '../MarkdownContent';

interface WorkflowWarningModalProps {
  isOpen: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  workflowDetails: {
    title?: string;
    description?: string;
    instructions?: string;
  };
  hasSecurityWarnings?: boolean;
}

/**
 * Asked before a workflow the person has not run before (or one carrying hidden characters) runs.
 *
 * A `ModalShell` at size `lg` with a scrolling body (spec 2.6; vision round 7, V3-R7-1 named it
 * among the raw `DialogContent` call sites): pinned at the top edge, the title in the header above
 * its rule, 16px insets, and the preview scrolling between the header and the footer rules. It
 * was a raw `DialogContent` 80% of the window wide, centred, with 24px insets. It must be answered
 * (`purpose="required"`): Escape and an outside press do nothing and there is no ×, as before.
 * The warning sentence is the dialog's description (`describedBy`), in body text.
 */
export function WorkflowWarningModal({
  isOpen,
  onConfirm,
  onCancel,
  workflowDetails,
  hasSecurityWarnings = false,
}: WorkflowWarningModalProps) {
  const descriptionId = React.useId();
  return (
    <ModalShell
      open={isOpen}
      // Required: nothing dismisses it. If anything ever did, cancelling is the safe answer.
      onOpenChange={(open) => !open && onCancel()}
      size="lg"
      purpose="required"
      scrollBody
      title={hasSecurityWarnings ? 'Security warning' : 'New workflow warning'}
      describedBy={descriptionId}
      bodyClassName="flex flex-col gap-3 py-3"
      footer={
        <>
          <Button variant="secondary" onClick={onCancel}>
            Cancel
          </Button>
          <Button onClick={onConfirm}>Trust and execute</Button>
        </>
      }
    >
      <p id={descriptionId} className="text-body text-text-muted">
        {!hasSecurityWarnings &&
          "You are about to execute a workflow that you haven't run before. "}
        Only proceed if you trust the source of this workflow.
      </p>

      {hasSecurityWarnings && (
        <Note tone="warning">
          This workflow contains hidden characters that will be ignored for your safety, as they
          could be used for malicious purposes.
        </Note>
      )}

      <div className="biorouter-modal-panel p-4 rounded-container">
        <h3 className="mb-3 text-label text-text-default">Workflow preview</h3>
        <div className="space-y-4">
          {workflowDetails.title && (
            <p className="text-body text-text-default">
              <strong className="font-semibold">Title</strong> {workflowDetails.title}
            </p>
          )}
          {workflowDetails.description && (
            <p className="text-body text-text-default">
              <strong className="font-semibold">Description</strong> {workflowDetails.description}
            </p>
          )}
          {workflowDetails.instructions && (
            <div>
              <h4 className="mb-1 text-label text-text-default">Instructions</h4>
              <MarkdownContent content={workflowDetails.instructions} className="text-body" />
            </div>
          )}
        </div>
      </div>
    </ModalShell>
  );
}
