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
    <div className="br-tool-preview-note flex items-center gap-1 px-3 py-1.5 text-supporting text-text-muted">
      <span>{PREVIEW_COPY.truncated}</span>
      <InfoTip label={PREVIEW_COPY.truncated} help={PREVIEW_COPY.truncatedHelp} />
    </div>
  );
}

/**
 * Shared chrome: a code box with a head row, after the code block recipe
 * (`.br-tool-preview`, tool-call.css). No visible edge and no rule under the
 * head (R-27): inside the filled approval card it steps back to the page
 * ground, a step of ground and not a fill on a fill.
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
    <div className="br-tool-preview">
      <div className="br-tool-preview-head flex items-center gap-2 px-3 py-1.5 text-supporting text-text-muted">
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

/**
 * Columns a wrapped line is estimated at, for deciding whether to collapse.
 * The narrowest card the preview is drawn in (a chat beside an open side
 * panel) holds about 70 characters of the 13px code face; under-estimating
 * only costs a "Show more" that opens a little.
 */
const WRAP_COLUMNS = 64;

/** Lines `text` takes once wrapped at {@link WRAP_COLUMNS}. */
function wrappedLines(text: string): number {
  return text
    .split('\n')
    .reduce((n, line) => n + Math.max(1, Math.ceil(line.length / WRAP_COLUMNS)), 0);
}

/**
 * The backend's pretty JSON as `[key, value]` pairs, or `null` when it is not
 * one whole object. A clipped blob is never parsed: what is shown must be what
 * was sent, and the "Truncated" note already says the rest is missing.
 */
function argumentFields(json: string, truncated: boolean): Array<[string, unknown]> | null {
  if (truncated) return null;
  try {
    const value: unknown = JSON.parse(json);
    if (value !== null && typeof value === 'object' && !Array.isArray(value)) {
      return Object.entries(value);
    }
  } catch {
    // Not JSON after all: shown as text below.
  }
  return null;
}

/**
 * A string is shown as itself, with real line breaks. Anything else (and an
 * empty string, which would otherwise read as a missing value) stays JSON, so
 * `["bug"]` is not mistaken for prose.
 */
function fieldText(value: unknown): string {
  return typeof value === 'string' && value !== '' ? value : JSON.stringify(value, null, 2);
}

/**
 * The arguments of a call, as fields a person can read.
 *
 * Final check, /bug (2026-10-12): shown as the backend's pretty JSON in a
 * `whitespace-pre` box, a string argument was one escaped line.
 * `platform__report_bug`'s body, the text that becomes a world-readable issue,
 * was 7,662 characters wide in a 724px box, about 90 of them visible, with
 * literal `\n`, under a card saying "Read the body below". So each field is
 * named, and its value is the exact text, wrapped (`.br-tool-preview-text`,
 * authored in tool-call.css so it holds wherever class scanning did not run).
 * The text is never rendered as Markdown: the person is checking what will be
 * published, and rendering hides some of it (an HTML comment, a link's URL).
 */
function ArgumentFields({ fields }: { fields: Array<[string, unknown]> }) {
  return (
    <dl className="br-tool-preview-fields">
      {fields.map(([key, value]) => (
        <div key={key} data-testid="tool-preview-field">
          <dt className="font-mono text-supporting text-text-muted">{key}</dt>
          <dd
            data-testid={`tool-preview-value-${key}`}
            className="br-tool-preview-text font-mono text-code text-text-default"
          >
            {fieldText(value)}
          </dd>
        </div>
      ))}
    </dl>
  );
}

function ArgumentsPreview({ json, truncated }: { json: string; truncated: boolean }) {
  const fields = argumentFields(json, truncated);
  // One key line per field, then its wrapped value.
  const lines = fields
    ? fields.reduce((n, [, value]) => n + 1 + wrappedLines(fieldText(value)), 0)
    : wrappedLines(json);
  return (
    <PreviewFrame
      icon={<Code className="size-3.5" />}
      title={PREVIEW_COPY.arguments}
      truncated={truncated}
      collapsible={lines > COLLAPSED_LINES}
    >
      {fields ? (
        <ArgumentFields fields={fields} />
      ) : (
        <pre
          data-testid="tool-preview-raw"
          className="br-tool-preview-text px-3 py-2 font-mono text-code text-text-default"
        >
          {json}
        </pre>
      )}
    </PreviewFrame>
  );
}

/** Render whatever the backend resolved this call into. */
export function ToolCallPreview({ preview }: { preview: ToolPreview }) {
  switch (preview.kind) {
    case 'folderAccess':
      return (
        <div data-testid="folder-access-preview" className="space-y-2 text-body">
          <p className="break-all font-mono">{preview.folder}</p>
          <p>
            {preview.access} access. A session grant includes subfolders and is not remembered after
            app restart.
          </p>
          <p>
            {preview.onceDescendants
              ? 'Allow once includes read-only analysis of this folder and its descendants for this operation:'
              : 'Allow once applies only to these files in this operation:'}
          </p>
          <ul className="list-disc pl-5">
            {preview.files.map((file) => (
              <li className="break-all font-mono" key={file}>
                {file}
              </li>
            ))}
          </ul>
        </div>
      );
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
      return <ArgumentsPreview json={preview.json} truncated={preview.truncated} />;

    default:
      // An older backend, or a preview kind this build does not know yet.
      // Showing nothing is correct — never invent a preview.
      return null;
  }
}

function countLines(text: string): number {
  return text.split('\n').length;
}
