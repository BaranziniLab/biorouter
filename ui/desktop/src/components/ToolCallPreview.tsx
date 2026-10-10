/**
 * BR-63: the "what will this actually do?" half of the tool-confirmation card.
 *
 * The backend (`conversation::tool_preview`) resolves a pending call into one of
 * four shapes — a shell command, a file diff, a new file, or a bag of arguments.
 * This renders them. Before BR-63 the card showed the tool's *name* and nothing
 * else, so "Allow?" was asked with no way to tell `ls` from `rm -rf`.
 *
 * Long previews collapse to a fixed window with a "Show more" toggle so a 200-line
 * diff cannot push the buttons off screen — the decision must always stay in reach.
 */
import { useId, useState } from 'react';
import './tool-call.css';
import { Code, FileText, Terminal } from './icons/app-icons';
import type { ActionRequired, ToolPreview, ToolPreviewLine, ToolRisk } from '../api';
import { Badge, type BadgeTone } from './ui/badge';
import { InfoTip } from './ui/info-tip';
import { PREVIEW_COPY, TOOL_ROW_COPY } from './toolCallCopy';

type ToolConfirmationData = Extract<ActionRequired['data'], { actionType: 'toolConfirmation' }>;
export type ToolConfirmationPreview = NonNullable<ToolConfirmationData['preview']>;

/** Lines shown before the preview collapses behind a "Show more" toggle. */
const COLLAPSED_LINES = 12;

const RISK_TONES: Record<ToolRisk, BadgeTone> = {
  low: 'success',
  medium: 'warning',
  high: 'danger',
  unknown: 'neutral',
};

/**
 * The BR-18 risk grade, in words. "Destructive" is a far more useful thing to put
 * in front of someone about to click "Always allow" than the tool's name.
 */
export function ToolRiskBadge({ risk }: { risk: ToolRisk }) {
  const known = risk in RISK_TONES;
  const grade: ToolRisk = known ? risk : 'unknown';
  return (
    <Badge data-testid="tool-risk-badge" tone={RISK_TONES[grade]}>
      {PREVIEW_COPY.risk[grade]}
    </Badge>
  );
}

/**
 * A clipped preview says so in one word; the explanation is an InfoTip. It is
 * never hidden altogether: a clipped diff must not pass for the whole edit.
 */
function TruncationNote() {
  return (
    <div className="flex items-center gap-1 border-t border-border-subtle px-3 py-1.5 text-supporting text-text-muted">
      <span>{PREVIEW_COPY.truncated}</span>
      <InfoTip label={PREVIEW_COPY.truncated} help={PREVIEW_COPY.truncatedHelp} />
    </div>
  );
}

/**
 * Shared chrome: a well (the ground fenced code sits on) with a head row over
 * a hairline, after Crew's code block. No border of its own, so inside the
 * approval card it is a step of ground, not a box in a box.
 */
function PreviewFrame({
  icon,
  title,
  machineTitle = false,
  meta,
  truncated,
  collapsible,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  /** A path: set in the code face. A label ("Command") stays in the UI face. */
  machineTitle?: boolean;
  meta?: React.ReactNode;
  truncated: boolean;
  collapsible: boolean;
  children: React.ReactNode;
}) {
  const [expanded, setExpanded] = useState(false);
  const bodyId = useId();

  return (
    <div className="overflow-hidden rounded-element bg-background-well">
      <div className="flex items-center gap-2 border-b border-border-subtle px-3 py-1.5 text-supporting text-text-muted">
        <span aria-hidden="true" className="shrink-0">
          {icon}
        </span>
        {/* A long path truncates; the whole path stays one hover away. */}
        <span
          className={machineTitle ? 'truncate font-mono' : 'truncate'}
          title={machineTitle ? title : undefined}
        >
          {title}
        </span>
        {meta && <span className="ml-auto shrink-0 tabular-nums">{meta}</span>}
      </div>

      <div
        id={bodyId}
        data-testid="tool-preview-body"
        // A collapsed preview is clipped, not unmounted: the toggle then only
        // changes height, and screen readers still see the whole call.
        className={`overflow-x-auto ${collapsible && !expanded ? 'max-h-52 overflow-y-hidden' : ''}`}
      >
        {children}
      </div>

      {collapsible && (
        <div className="px-3 pb-2">
          <button
            type="button"
            className="br-tool-more"
            onClick={() => setExpanded((v) => !v)}
            aria-expanded={expanded}
            aria-controls={bodyId}
          >
            {expanded ? TOOL_ROW_COPY.showLess : TOOL_ROW_COPY.showMore}
          </button>
        </div>
      )}

      {truncated && <TruncationNote />}
    </div>
  );
}

const LINE_STYLES: Record<ToolPreviewLine['kind'], { marker: string; className: string }> = {
  added: { marker: '+', className: 'bg-background-success/10 text-text-success' },
  removed: { marker: '-', className: 'bg-background-danger/10 text-text-danger' },
  context: { marker: ' ', className: 'text-text-muted' },
};

function DiffLines({ lines }: { lines: ToolPreviewLine[] }) {
  return (
    <pre className="py-2 font-mono text-code">
      {lines.map((line, i) => {
        const style = LINE_STYLES[line.kind] ?? LINE_STYLES.context;
        return (
          <div key={i} data-testid={`diff-line-${line.kind}`} className={`px-3 ${style.className}`}>
            <span aria-hidden="true" className="select-none opacity-60">
              {style.marker}{' '}
            </span>
            {line.text}
          </div>
        );
      })}
    </pre>
  );
}

function CodeBlock({ text }: { text: string }) {
  return (
    <pre className="whitespace-pre px-3 py-2 font-mono text-code text-text-default">{text}</pre>
  );
}

/** Render whatever the backend resolved this call into. */
export function ToolCallPreview({ preview }: { preview: ToolPreview }) {
  switch (preview.kind) {
    case 'shell':
      return (
        <PreviewFrame
          icon={<Terminal className="size-3.5" />}
          title={PREVIEW_COPY.command}
          truncated={preview.truncated}
          collapsible={countLines(preview.command) > COLLAPSED_LINES}
        >
          <CodeBlock text={preview.command} />
        </PreviewFrame>
      );

    case 'fileEdit':
      return (
        <PreviewFrame
          icon={<FileText className="size-3.5" />}
          title={preview.path}
          machineTitle
          meta={
            <span>
              <span className="text-text-success">+{preview.added}</span>{' '}
              <span className="text-text-danger">-{preview.removed}</span>
            </span>
          }
          truncated={preview.truncated}
          collapsible={preview.lines.length > COLLAPSED_LINES}
        >
          <DiffLines lines={preview.lines} />
        </PreviewFrame>
      );

    case 'fileWrite':
      return (
        <PreviewFrame
          icon={<FileText className="size-3.5" />}
          title={preview.path}
          machineTitle
          meta={
            <span className="text-text-success">{PREVIEW_COPY.newFile(preview.lineCount)}</span>
          }
          truncated={preview.truncated}
          collapsible={preview.lineCount > COLLAPSED_LINES}
        >
          <CodeBlock text={preview.content} />
        </PreviewFrame>
      );

    case 'arguments':
      return (
        <PreviewFrame
          icon={<Code className="size-3.5" />}
          title={PREVIEW_COPY.arguments}
          truncated={preview.truncated}
          collapsible={countLines(preview.json) > COLLAPSED_LINES}
        >
          <CodeBlock text={preview.json} />
        </PreviewFrame>
      );

    default:
      // An older backend, or a preview kind this build does not know yet.
      // Showing nothing is correct — never invent a preview.
      return null;
  }
}

function countLines(text: string): number {
  return text.split('\n').length;
}
