import MarkdownContent from '../MarkdownContent';
import { ToolContentPreview } from '../ToolContentPreview';
import { TranscriptRow, TranscriptRowSection } from '../TranscriptRow';
import type { ArtifactSource } from '../artifacts/artifactTypes';
import { toolGlyphFor } from '../../utils/toolGlyph';
import { transcriptCopy } from './copy';
import type { SubagentResultView } from './subagentResult';

const copy = transcriptCopy.subagentResult;

/** The glyph the delegation row draws, so a subagent's start and its result read as one thing. */
const SUBAGENT_GLYPH = toolGlyphFor('workspace__subagent');

const count = (value: number) => value.toLocaleString();

/**
 * The well's voice for what a tool or a subagent returned: the code role, as
 * the tool rows' output, input and failure text read (`transcript-row.css`).
 */
const VALUE_TEXT =
  'whitespace-pre-wrap break-words font-mono text-code text-text-default [overflow-wrap:anywhere]';

function ProseSection({
  label,
  text,
  onOpenArtifact,
  workingDir,
}: {
  label: string;
  text: string;
  onOpenArtifact: (artifact: ArtifactSource) => void;
  workingDir?: string;
}) {
  return (
    <TranscriptRowSection label={label}>
      <ToolContentPreview text={text}>
        {(visible, truncated) =>
          truncated ? (
            <pre className={VALUE_TEXT}>{visible}</pre>
          ) : (
            <MarkdownContent
              content={visible}
              className="max-w-full overflow-x-auto"
              onOpenArtifact={onOpenArtifact}
              workingDir={workingDir}
            />
          )
        }
      </ToolContentPreview>
    </TranscriptRowSection>
  );
}

function PlainSection({ label, text }: { label: string; text: string }) {
  return (
    <TranscriptRowSection label={label}>
      <p className={VALUE_TEXT}>{text}</p>
    </TranscriptRowSection>
  );
}

/**
 * A delegated subagent's result in its parent's chat: one transcript line in
 * the tool rows' look ("Subagent finished: Read every trial report"), with
 * what it said behind the line's disclosure. Never the injection frame, the
 * trust paragraph, raw JSON or a session id: those are for the model, which
 * still receives them (`readSubagentResult` explains the message's shape).
 */
export function SubagentResultRow({
  result,
  onOpenArtifact,
  workingDir,
}: {
  result: SubagentResultView;
  onOpenArtifact: (artifact: ArtifactSource) => void;
  workingDir?: string;
}) {
  const outcome = result.outcome ? copy.outcome[result.outcome] : copy.ended;
  const failed = result.outcome === 'error';
  // A failure's summary is "Subagent failed: <error>", which the Error
  // section already says; any other summary is the subagent's own words.
  const summary =
    result.summary && !(failed && result.error && result.summary.endsWith(result.error))
      ? result.summary
      : null;
  const monitorLine = result.monitor
    ? copy.monitorLine(result.monitor.name, result.monitor.question)
    : '';

  const sections = [
    result.error && <PlainSection key="error" label={copy.error} text={result.error} />,
    result.question && <PlainSection key="question" label={copy.question} text={result.question} />,
    summary && (
      <ProseSection
        key="summary"
        label={copy.summary}
        text={summary}
        onOpenArtifact={onOpenArtifact}
        workingDir={workingDir}
      />
    ),
    monitorLine && <PlainSection key="monitor" label={copy.monitor} text={monitorLine} />,
    result.files.length > 0 && (
      <TranscriptRowSection key="files" label={copy.files}>
        <ul className="flex min-w-0 flex-col gap-1">
          {result.files.map((file, index) => (
            <li key={`${index}:${file}`} className={VALUE_TEXT}>
              {file}
            </li>
          ))}
        </ul>
      </TranscriptRowSection>
    ),
    result.decisions && (
      <PlainSection key="decisions" label={copy.decisions} text={result.decisions} />
    ),
    result.tokens && (
      <PlainSection
        key="tokens"
        label={copy.tokens}
        text={copy.tokenLine(
          count(result.tokens.total),
          count(result.tokens.input),
          count(result.tokens.output)
        )}
      />
    ),
    result.humanIntervened && (
      <p key="intervened" className="text-supporting text-text-muted">
        {copy.intervened}
      </p>
    ),
    result.raw && (
      <TranscriptRowSection key="raw" label={copy.result}>
        <ToolContentPreview text={result.raw}>
          {(visible) => <pre className={VALUE_TEXT}>{visible}</pre>}
        </ToolContentPreview>
      </TranscriptRowSection>
    ),
  ].filter(Boolean);

  return (
    // `biorouter-message-tool` is the tool rows' own wrapper: the same entrance
    // when it arrives live, and the same room left for an answer's floating
    // actions when one sits right under it (`message-meta.css`).
    <div className="biorouter-message-tool">
      <TranscriptRow
        icon={SUBAGENT_GLYPH.Icon}
        glyph={SUBAGENT_GLYPH.kind}
        label={copy.label(outcome, result.title)}
        tone={failed ? 'danger' : 'muted'}
        triggerClassName="br-tool-disclosure"
        data-testid="subagent-result-row"
      >
        {sections.length > 0 ? sections : undefined}
      </TranscriptRow>
    </div>
  );
}
