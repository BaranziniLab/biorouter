import { describe, expect, it } from 'vitest';
import {
  personFacingToolError,
  repetitionStopNotice,
  withoutToolErrorHeader,
} from './repetitionStop';

// The daemon's four hard-stop reasons, verbatim as `tool_monitor.rs` writes them
// for the agent (and an older daemon sent as the turn's ending).
const EXACT =
  "Biorouter stopped this tool call: 'decisions__decide' has now been called with identical " +
  'arguments 5 times without a changed result. The user did NOT decline it; this is an ' +
  'automatic repetition guard. Repeating the same call will not produce a different result. ' +
  'Change approach: vary the arguments, use a different tool, or explain what is blocking you ' +
  'and stop.';
const NEAR_DUP =
  "Biorouter stopped this tool call: 'decisions__decide' has been called 6 times in a row with " +
  'near-identical arguments (only minor tweaks). The user did NOT decline it; this is an ' +
  'automatic loop guard. Change approach: use a different tool, re-read the last result, or ' +
  'explain what is blocking you and stop.';
const OSCILLATION =
  'Biorouter stopped this tool call: your last 8 tool calls alternate between the same two ' +
  "calls (…the most recent being 'decisions__decide'), each with unchanged arguments. The user " +
  'did NOT decline it; this is an automatic loop guard. Break the cycle: try a different ' +
  'approach, or explain what is blocking you and stop.';
const FAILURE =
  "Biorouter did not run this tool call: 'decisions__decide' has already failed 3 times in a " +
  'row with the same error, and the warnings changed nothing. The user did NOT decline it; this ' +
  'is an automatic no-progress guard.';

const HEADER =
  '[tool_error kind=transient retryable=true] A dependency blipped. One retry is reasonable.';

describe('repetitionStopNotice', () => {
  it.each([
    [EXACT, 'Biorouter stopped the agent repeating the same step (decide, 5 times).'],
    [NEAR_DUP, 'Biorouter stopped the agent repeating nearly the same step (decide, 6 times).'],
    [
      OSCILLATION,
      'Biorouter stopped the agent going back and forth between the same two steps (decide, 8 calls).',
    ],
    [
      FAILURE,
      'Biorouter stopped the agent retrying a step that keeps failing the same way (decide, 3 times).',
    ],
  ])('turns the agent-addressed stop into one plain sentence', (reason, notice) => {
    expect(repetitionStopNotice(reason)).toBe(notice);
    expect(repetitionStopNotice(`${HEADER}\n${reason}`)).toBe(notice);
  });

  it("keeps the daemon's own sentence as it is", () => {
    const notice = 'Biorouter stopped the agent repeating the same step (decide, 5 times).';
    expect(repetitionStopNotice(notice)).toBe(notice);
  });

  it('is null for anything that is not a repetition stop', () => {
    expect(repetitionStopNotice('Error: No matches found for: web search')).toBeNull();
    expect(repetitionStopNotice('')).toBeNull();
  });
});

describe('withoutToolErrorHeader', () => {
  it("drops the header line, guidance and all, and keeps the tool's own words", () => {
    expect(withoutToolErrorHeader(`${HEADER}\nError: connection reset by peer`)).toBe(
      'Error: connection reset by peer'
    );
    expect(
      withoutToolErrorHeader(
        '[tool_error kind=not_found retryable=false at=src/main.rs:42] The target does not exist. ' +
          'Locate the right one before calling again.\nNo such file: src/main.rs'
      )
    ).toBe('No such file: src/main.rs');
  });

  it('keeps the guidance when the header is all there is', () => {
    expect(withoutToolErrorHeader(HEADER)).toBe('A dependency blipped. One retry is reasonable.');
  });

  it('leaves text without a header untouched', () => {
    const text = 'Error: see [tool_error kind=x] in the log';
    expect(withoutToolErrorHeader(text)).toBe(text);
  });
});

describe('personFacingToolError', () => {
  it('shows a repetition stop as its sentence, never the raw slug or the agent text', () => {
    const shown = personFacingToolError(`${HEADER}\n${EXACT}`);
    expect(shown).toBe('Biorouter stopped the agent repeating the same step (decide, 5 times).');
    for (const raw of ['[tool_error', 'retryable=', 'did NOT decline', 'decisions__decide']) {
      expect(shown).not.toContain(raw);
    }
  });

  it('shows any other failure without its header', () => {
    expect(personFacingToolError(`${HEADER}\nError: the decision server failed`)).toBe(
      'Error: the decision server failed'
    );
  });
});
