import { describe, expect, it } from 'vitest';
import { briefSelectionFailure } from './selectionWarning';

/**
 * F14. A request naming a private chat on `GET /knowledge/active` with neither
 * the user's proof nor a private model is refused with a paragraph addressed to
 * an AI agent. The refusal stays exactly as it is; what was wrong is that it was
 * printed in full in a devtools console, whose only reader is a person.
 *
 * (F14 took the desktop app's refusal for a correct one, on a chat bound to a
 * public model. It was not: the desktop's read carried no proof, and was refused
 * for every private chat — see `KnowledgeContext.tsx`.)
 */
const AGENT_REFUSAL =
  'That chat is private, or there is no chat with that id. This request was made on a public ' +
  'model and carried no proof it came from the person at the keyboard, and the two answers are ' +
  'deliberately the same so that nothing about the chat is disclosed. Nothing was read and ' +
  'nothing was changed. Do not retry as you are; the same call will be refused again, and no ' +
  'setting, hook or permission mode changes it. A private chat is reachable from a session ' +
  'running a private model, one the institution hosts or one that runs on this machine, or ' +
  'from the desktop app when the person at the keyboard acts. Pointing a program that already ' +
  'runs under such a model at this daemon is a setup decision for whoever operates it, and the ' +
  'Biorouter documentation covers it under ‘Reaching a private chat from a script’. If ' +
  'this task genuinely needs that chat, stop and ask the user to open it for you.';

describe('briefSelectionFailure', () => {
  it('keeps the first sentence of the agent-facing refusal and drops the rest', () => {
    const brief = briefSelectionFailure(new Error(AGENT_REFUSAL));
    expect(brief).toBe('That chat is private, or there is no chat with that id.');
    // The instructions are the part that reads as a crash to a person, and the
    // part that is addressed to somebody who is not there.
    expect(brief).not.toContain('Do not retry as you are');
    expect(brief).not.toContain('ask the user to open it for you');
    expect(brief.length).toBeLessThan(AGENT_REFUSAL.length / 4);
  });

  /**
   * Trimming to the first SENTENCE rather than to a fixed prefix is what keeps
   * this useful for the failures that are not the refusal.
   */
  it('leaves a short real failure intact', () => {
    expect(briefSelectionFailure(new TypeError('Failed to fetch'))).toBe('Failed to fetch');
    expect(briefSelectionFailure({ message: 'Internal Server Error' })).toBe(
      'Internal Server Error'
    );
  });

  it('caps a first sentence that is itself a paragraph', () => {
    const long = `${'x'.repeat(400)}. and then some`;
    const brief = briefSelectionFailure(new Error(long));
    expect(brief).toHaveLength(160);
    expect(brief.endsWith('…')).toBe(true);
  });

  /**
   * ⚠ Measured in the running app: this route's body is `text/plain`, so the
   * generated client throws a bare STRING. Routing that through
   * `conversionUtils.errorMessage(err, 'unknown error')` returns the DEFAULT
   * for a string, which printed `Knowledge selection not hydrated: unknown
   * error` and threw away the only fact the line carries.
   */
  it('reads a bare string body, which is what this route actually throws', () => {
    expect(briefSelectionFailure(AGENT_REFUSAL)).toBe(
      'That chat is private, or there is no chat with that id.'
    );
  });

  it('reads the JSON shapes the other routes throw', () => {
    expect(briefSelectionFailure({ error: 'no such base' })).toBe('no such base');
    expect(briefSelectionFailure({ error: { message: 'no such base' } })).toBe('no such base');
    expect(briefSelectionFailure({ detail: 'no such base' })).toBe('no such base');
  });

  it('never returns an empty line, whatever it is handed', () => {
    expect(briefSelectionFailure(undefined)).toBe('unknown error');
    expect(briefSelectionFailure(new Error(''))).toBe('unknown error');
    // An object with no text anywhere is not a shape this can improve on.
    expect(briefSelectionFailure({})).toBe('unknown error');
    expect(briefSelectionFailure('   ')).toBe('unknown error');
  });

  it('collapses newlines so one warning stays one line', () => {
    expect(briefSelectionFailure(new Error('refused\n\n  because of reasons'))).toBe(
      'refused because of reasons'
    );
  });
});
