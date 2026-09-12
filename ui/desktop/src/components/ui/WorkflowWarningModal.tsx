import {
  Dialog,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogPortal,
  DialogOverlay,
} from './dialog';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import { Button } from './button';
import MarkdownContent from '../MarkdownContent';
import { cn } from '../../utils';

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
      <DialogPortal>
        <DialogOverlay />
        <DialogPrimitive.Content
          className={cn(
            // z-[var(--z-modal)] must stay ABOVE the sibling <DialogOverlay/>
            // (--z-overlay). A stale z-50 here put the full-screen, click-eating
            // scrim on top of this modal's own buttons and the rest of the app,
            // soft-locking it. See the .biorouter-modal-surface floor in main.css.
            'biorouter-modal-surface bg-background-default data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 fixed top-[50%] left-[50%] z-[var(--z-modal)] grid w-full max-w-[calc(100%-2rem)] translate-x-[-50%] translate-y-[-50%] gap-4 duration-200 sm:max-w-[80vw] max-h-[80vh] flex flex-col p-0 overflow-hidden'
          )}
          onPointerDownOutside={(e) => e.preventDefault()}
          onEscapeKeyDown={(e) => e.preventDefault()}
        >
          <DialogHeader className="flex-shrink-0 p-6 pb-0">
            <DialogTitle>
              {hasSecurityWarnings ? '⚠️ Security Warning' : '⚠️ New Workflow Warning'}
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
                        This workflow contains hidden characters that will be ignored for your
                        safety, as they could be used for malicious purposes.
                      </p>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          )}

          <div className="flex-1 overflow-y-auto p-6 pt-4">
            <div className="biorouter-modal-panel p-4 rounded-container">
              <h3 className="font-medium mb-3 text-text-default">Workflow Preview:</h3>
              <div className="space-y-4">
                {workflowDetails.title && (
                  <p className="text-text-default">
                    <strong>Title:</strong> {workflowDetails.title}
                  </p>
                )}
                {workflowDetails.description && (
                  <p className="text-text-default">
                    <strong>Description:</strong> {workflowDetails.description}
                  </p>
                )}
                {workflowDetails.instructions && (
                  <div>
                    <h4 className="font-medium text-text-default mb-1">Instructions:</h4>
                    <MarkdownContent content={workflowDetails.instructions} className="text-body" />
                  </div>
                )}
              </div>
            </div>
          </div>

          <DialogFooter className="flex-shrink-0 p-6 pt-0">
            <Button variant="outline" onClick={onCancel}>
              Cancel
            </Button>
            <Button onClick={onConfirm}>Trust and execute</Button>
          </DialogFooter>
        </DialogPrimitive.Content>
      </DialogPortal>
    </Dialog>
  );
}
