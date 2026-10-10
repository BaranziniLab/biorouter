import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './dialog';
import { Button } from './button';
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

export function WorkflowWarningModal({
  isOpen,
  onConfirm,
  onCancel,
  workflowDetails,
  hasSecurityWarnings = false,
}: WorkflowWarningModalProps) {
  return (
    <Dialog open={isOpen}>
      {/* The shared `DialogContent`, so this dialog opens and closes with the same motion as
          every other, returns focus to the control that opened it, and is announced as modal.
          It was a hand-assembled `DialogPrimitive.Content` with `zoom-in-95` and `duration-200`.
          Not dismissible: the person has to answer (Escape and an outside press do nothing, and
          there is no close button), as before. */}
      <DialogContent
        dismissible={false}
        className="flex max-h-[80vh] flex-col gap-4 overflow-hidden p-0 sm:max-w-[80vw]"
      >
        <DialogHeader className="flex-shrink-0 p-6 pb-0">
          <DialogTitle>
            {hasSecurityWarnings ? 'Security warning' : 'New workflow warning'}
          </DialogTitle>
          <DialogDescription>
            {!hasSecurityWarnings &&
              "You are about to execute a workflow that you haven't run before. "}
            Only proceed if you trust the source of this workflow.
          </DialogDescription>
        </DialogHeader>

        {hasSecurityWarnings && (
          <div className="px-6">
            <div className="rounded-container border border-border-warning/40 bg-background-warning/10 p-4">
              <div className="flex items-start">
                <div className="ml-3">
                  <div className="mt-2 text-body text-text-warning">
                    <p>
                      This workflow contains hidden characters that will be ignored for your safety,
                      as they could be used for malicious purposes.
                    </p>
                  </div>
                </div>
              </div>
            </div>
          </div>
        )}

        <div className="flex-1 overflow-y-auto p-6 pt-4">
          <div className="biorouter-modal-panel p-4 rounded-container">
            <h3 className="mb-3 text-label text-text-default">Workflow preview</h3>
            <div className="space-y-4">
              {workflowDetails.title && (
                <p className="text-text-default">
                  <strong>Title</strong> {workflowDetails.title}
                </p>
              )}
              {workflowDetails.description && (
                <p className="text-text-default">
                  <strong>Description</strong> {workflowDetails.description}
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
        </div>

        <DialogFooter className="flex-shrink-0 p-6 pt-0">
          <Button variant="secondary" onClick={onCancel}>
            Cancel
          </Button>
          <Button onClick={onConfirm}>Trust and execute</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
