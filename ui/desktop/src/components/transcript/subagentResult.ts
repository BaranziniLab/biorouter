import type { Message } from '../../api';
import { getTextContent } from '../../types/message';
import { sanitizeUntrustedLabel, stripHiddenCharacters } from '../../utils/untrustedText';

/**
 * A delegated subagent's result as the parent chat's transcript shows it.
 *
 * When a background subagent reaches a terminal result, the daemon appends one
 * assistant message to the parent chat (`native_supervision_message` in
 * `crates/biorouter/src/agents/agent.rs`). Its text is written for the MODEL:
 *
 *     Delegated subagent <title> (<child session id>, generation <n>) reached a terminal result:
 *     <workspace-injection untrusted="true" from="<title>">
 *     <a paragraph telling the model to treat the text as lower-trust data>
 *     <the SubagentResult envelope as pretty JSON>
 *     </workspace-injection>
 *
 * Drawn as prose, that put the frame, the trust paragraph, a session id and raw
 * JSON in front of the person. This module reads it back into the parts a
 * person needs (who, how it ended, what it said), so the transcript can draw it
 * as one row. The model still receives the text unchanged.
 */

/** How a subagent run ended: `SubagentStatus` in `agents/subagent_result.rs`. */
export type SubagentOutcome = 'completed' | 'blocked' | 'incomplete' | 'error';

const OUTCOMES: ReadonlySet<string> = new Set<SubagentOutcome>([
  'completed',
  'blocked',
  'incomplete',
  'error',
]);

export interface SubagentResultView {
  /** The subagent's title (its task, one line), or null when none can be read. */
  title: string | null;
  /** Null when the result could not be read; the row then says only that it ended. */
  outcome: SubagentOutcome | null;
  summary: string | null;
  error: string | null;
  /** The question a blocked subagent returned instead of guessing. */
  question: string | null;
  /** Files the subagent created or changed. */
  files: string[];
  tokens: { total: number; input: number; output: number } | null;
  /** The person typed into the subagent's chat while it ran. */
  humanIntervened: boolean;
  /** A decision-model monitor stopped the run: its name and question, never its id. */
  monitor: { name: string; question: string } | null;
  /** The subagent's last decision-model marker text. */
  decisions: string | null;
  /**
   * The result's text with the frame, the trust paragraph and the header line
   * (which holds the session id) removed. Set only when the result could not be
   * read, so the row can still offer what arrived.
   */
  raw: string | null;
}

const PREFIX = 'Delegated subagent ';
/**
 * The header line. The title sits between the prefix and the LAST
 * parenthesised `(<session id>, generation <n>)`, so a title that holds
 * parentheses of its own still reads whole.
 */
const HEADER =
  /^Delegated subagent ([^\n]*) \([^()\n]*, generation \d+\) reached a terminal result:/;
const FRAME_OPEN = /<workspace-injection\b[^>]*>/i;
const FRAME_CLOSE = /<\/workspace-injection\s*>/gi;
/** The first line of the trust paragraph `frame_workspace_injection` writes. */
const TRUST_NOTE = /^The text below was sent by an agent running in /;

/**
 * Whether a message is a subagent's result notice: an agent-injected assistant
 * message made of text only, opening with the daemon's header. The renderer
 * and the transcript's rhythm both ask this, so they cannot disagree about
 * which messages draw as the row.
 */
export function isSubagentResultMessage(message: Message): boolean {
  if (message.role !== 'assistant') return false;
  if (message.metadata?.provenance?.kind !== 'agent_injection') return false;
  if (message.content.length === 0) return false;
  if (!message.content.every((content) => content.type === 'text')) return false;
  return getTextContent(message).trimStart().startsWith(PREFIX);
}

const textField = (value: unknown): string | null => {
  if (typeof value !== 'string') return null;
  const text = value.trim();
  return text ? text : null;
};

const countField = (value: unknown): number | null =>
  typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : null;

function recordOf(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/** The frame's body, without the frame. Null when no frame opens. */
function framedBody(text: string): string | null {
  const open = FRAME_OPEN.exec(text);
  if (!open) return null;
  const start = open.index + open[0].length;
  const rest = text.slice(start);
  // The LAST close: the daemon neutralises any close tag inside the body, so the
  // frame's own close is the final one.
  let end = -1;
  for (const match of rest.matchAll(FRAME_CLOSE)) end = match.index ?? end;
  return end === -1 ? rest : rest.slice(0, end);
}

/** The body without the trust paragraph that opens it. */
function withoutTrustNote(body: string): string {
  const lines = body.split('\n');
  const first = lines.findIndex((line) => line.trim() !== '');
  if (first !== -1 && TRUST_NOTE.test(lines[first].trim())) lines.splice(first, 1);
  return lines.join('\n').trim();
}

/** What arrived, for a result that could not be read: never the header, the frame or the note. */
function rawText(text: string, body: string | null): string {
  const content = body ?? text.replace(HEADER, '').replace(FRAME_OPEN, '').replace(FRAME_CLOSE, '');
  return stripHiddenCharacters(withoutTrustNote(content).replace(FRAME_CLOSE, ''))
    .replace(/\r/g, '')
    .trim();
}

function parseEnvelope(body: string): Record<string, unknown> | null {
  const start = body.indexOf('{');
  const end = body.lastIndexOf('}');
  if (start === -1 || end <= start) return null;
  try {
    return recordOf(JSON.parse(body.slice(start, end + 1)));
  } catch {
    // A body capped by `cap_workspace_injection` (head, marker, tail) is not
    // JSON any more; the row falls back to showing that it ended.
    return null;
  }
}

/**
 * Read a subagent's result notice. Null for any other message. Defensive by
 * design: every field is checked, and a body that is not the envelope still
 * yields a row ("Subagent finished") with what arrived behind its disclosure.
 */
export function readSubagentResult(message: Message): SubagentResultView | null {
  if (!isSubagentResultMessage(message)) return null;
  const text = getTextContent(message).trimStart();

  const fromProvenance = message.metadata?.provenance?.fromSessionName;
  const fromHeader = HEADER.exec(text)?.[1];
  const title =
    sanitizeUntrustedLabel(fromProvenance ?? '') ||
    sanitizeUntrustedLabel(fromHeader ?? '') ||
    null;

  const body = framedBody(text);
  const envelope = body !== null ? parseEnvelope(withoutTrustNote(body)) : null;
  const status = envelope?.status;
  const outcome =
    typeof status === 'string' && OUTCOMES.has(status) ? (status as SubagentOutcome) : null;

  if (!envelope || !outcome) {
    return {
      title,
      outcome: null,
      summary: envelope ? textField(envelope.summary) : null,
      error: envelope ? textField(envelope.error) : null,
      question: null,
      files: [],
      tokens: null,
      humanIntervened: false,
      monitor: null,
      decisions: null,
      raw: rawText(text, body) || null,
    };
  }

  const files = Array.isArray(envelope.artifacts)
    ? envelope.artifacts
        .map((file) => (typeof file === 'string' ? stripHiddenCharacters(file).trim() : ''))
        .filter(Boolean)
    : [];

  const tokenRecord = recordOf(envelope.tokens);
  const total = countField(tokenRecord?.total);
  const input = countField(tokenRecord?.input);
  const output = countField(tokenRecord?.output);
  const tokens =
    total !== null && input !== null && output !== null ? { total, input, output } : null;

  const monitorRecord = recordOf(envelope.stopped_by_monitor);
  const monitorName = textField(monitorRecord?.name);
  const monitor = monitorRecord
    ? {
        name: monitorName ? sanitizeUntrustedLabel(monitorName) : '',
        question: textField(monitorRecord.question) ?? '',
      }
    : null;

  return {
    title,
    outcome,
    summary: textField(envelope.summary),
    error: textField(envelope.error),
    question: textField(envelope.question),
    files,
    tokens,
    humanIntervened: envelope.human_intervened === true,
    monitor,
    decisions: textField(envelope.decisions),
    raw: null,
  };
}
