import { transcriptCopy } from './transcript/copy';

interface WorkflowHeaderProps {
  title: string;
}

/**
 * The line above a workflow chat: "Workflow {title}" in 13px, the kind muted
 * and the name in full ink, on the column's own left edge. A line, not a band:
 * no rule under it and no inset of its own.
 */
export function WorkflowHeader({ title }: WorkflowHeaderProps) {
  return (
    <div className="flex h-8 min-w-0 items-center gap-1 text-secondary">
      <span className="flex-none text-text-muted">{transcriptCopy.workflow.label}</span>
      <span className="min-w-0 truncate text-text-default">{title}</span>
    </div>
  );
}
