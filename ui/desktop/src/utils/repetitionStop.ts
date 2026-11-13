/**
 * The loop guard's stop, and a failed tool call's error, as the PERSON reads
 * them.
 *
 * Two texts the daemon writes for the agent were reaching the person verbatim
 * (live QA, 2026-11-11, five identical `decisions__decide` calls):
 *
 * - the repetition guard's stop ("Biorouter stopped this tool call:
 *   'decisions__decide' has now been called with identical arguments 5 times…
 *   The user did NOT decline it…"), under a red "Model request failed";
 * - the error taxonomy's header (`[tool_error kind=transient retryable=true]
 *   <advice to the agent>`, `crates/biorouter/src/agents/tool_errors.rs`),
 *   printed at the top of a failed tool row.
 *
 * The agent keeps both: they are its tool result. What the person sees is one
 * plain sentence for a stop and the tool's own words for an error.
 */

/** The wire code of a turn the repetition guard ended (`TurnAbortCode::ToolLoop`). */
export const TOOL_LOOP_CODE = 'tool_loop';

/** The turn row's title for a repetition stop: a notice, not a failure. */
export const REPETITION_STOP_TITLE = 'Stopped a repeated step';

/** The sentence when nothing names the step. */
export const REPETITION_STOP_FALLBACK = 'Biorouter stopped the agent repeating the same step.';

/**
 * The first line of an annotated error: the bracketed header, then the
 * taxonomy's guidance to the agent on the same line.
 */
const TOOL_ERROR_HEADER = /^\s*\[tool_error [^\]\n]*\][ \t]*([^\n]*)(?:\n([\s\S]*))?$/;

/**
 * `text` without the taxonomy's machine header. The header is one line put
 * in front of the tool's own text; with that text present the whole line goes,
 * guidance included, because the guidance is addressed to the agent. A result
 * that was only the header keeps the guidance, which is still a sentence.
 */
export function withoutToolErrorHeader(text: string): string {
  const match = TOOL_ERROR_HEADER.exec(text);
  if (!match) return text;
  const original = (match[2] ?? '').trim();
  if (original) return original;
  return match[1].trim() || text;
}

/** The short name a tool row uses: `decide`, not `decisions__decide`. */
function stepName(tool: string): string {
  const parts = tool.split('__');
  return parts[parts.length - 1] || tool;
}

/**
 * The daemon's four stop reasons (`crates/biorouter/src/tool_monitor.rs`), as
 * an older daemon still sends them, and the sentence each becomes. The daemon
 * writes the same sentences itself now (`repetition_stop_notice`); keep the
 * two in step.
 */
const LEGACY_STOPS: ReadonlyArray<{
  pattern: RegExp;
  what: string;
  unit: 'times' | 'calls';
}> = [
  {
    pattern:
      /^Biorouter stopped this tool call: '([^']+)' has now been called with identical arguments (\d+) times/,
    what: 'repeating the same step',
    unit: 'times',
  },
  {
    pattern:
      /^Biorouter stopped this tool call: '([^']+)' has been called (\d+) times in a row with near-identical arguments/,
    what: 'repeating nearly the same step',
    unit: 'times',
  },
  {
    pattern:
      /^Biorouter stopped this tool call: your last (\d+) tool calls alternate between the same two calls \(…the most recent being '([^']+)'\)/,
    what: 'going back and forth between the same two steps',
    unit: 'calls',
  },
  {
    pattern:
      /^Biorouter did not run this tool call: '([^']+)' has already failed (\d+) times in a row/,
    what: 'retrying a step that keeps failing the same way',
    unit: 'times',
  },
];

/** What a current daemon sends: already the person's sentence. */
const PERSON_NOTICE = /^Biorouter stopped the agent /;

/**
 * The plain sentence for a repetition stop, or `null` when `text` is not one.
 * Accepts the daemon's own sentence, and the agent-addressed reason an older
 * daemon (or a stored tool result) carries.
 */
export function repetitionStopNotice(text: string): string | null {
  const trimmed = withoutToolErrorHeader(text).trim();
  if (PERSON_NOTICE.test(trimmed)) return trimmed.split('\n')[0].trim();
  for (const { pattern, what, unit } of LEGACY_STOPS) {
    const match = pattern.exec(trimmed);
    if (!match) continue;
    // The oscillation reason names the count before the tool.
    const [tool, run] = unit === 'calls' ? [match[2], match[1]] : [match[1], match[2]];
    return `Biorouter stopped the agent ${what} (${stepName(tool)}, ${run} ${unit}).`;
  }
  return null;
}

/** A failed tool call's error text as the person reads it. */
export function personFacingToolError(text: string): string {
  return repetitionStopNotice(text) ?? withoutToolErrorHeader(text);
}
