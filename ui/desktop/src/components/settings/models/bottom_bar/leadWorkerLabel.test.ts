import { describe, expect, it } from 'vitest';
import {
  DEFAULT_LEAD_TURNS,
  leadWorkerActive,
  leadWorkerChip,
  leadWorkerHandoverNote,
  readLeadTurns,
  type LeadWorkerPair,
} from './leadWorkerLabel';

/**
 * D7 of the 2026-09-12 model-controls run, as arithmetic.
 *
 * React-free and DOM-free on purpose, the rule `utils/messageClamp.ts` and
 * `styles/measures.test.ts` already record: a threshold you can only exercise by
 * rendering a component is one nobody re-tests. The rendered half — that the chip
 * prints what this returns — is `ModelsBottomBar.leadWorker.test.tsx`.
 *
 * Measured live against `origin/main` (dev GUI, sandboxed config, 2026-09-12):
 * lead `claude_code / claude-opus-5`, worker `versa_azure /
 * gpt-4.1-mini-2025-04-14`, `BIOROUTER_LEAD_TURNS: 3`. Home's chip read
 * `Current model: gpt-4.1-mini-2025-04-14 (worker) (Public model)` — the half
 * that does not answer the next message, under the word that says it does.
 */

const PAIR: LeadWorkerPair = {
  leadModel: 'claude-opus-5',
  leadProvider: 'claude_code',
  workerModel: 'gpt-4.1-mini-2025-04-14',
  leadTurns: 3,
};
const NO_PAIR: LeadWorkerPair = {
  leadModel: '',
  leadProvider: '',
  workerModel: 'gpt-5.5-2026-04-24',
  leadTurns: DEFAULT_LEAD_TURNS,
};

describe('leadWorkerChip', () => {
  it('names the lead, as the lead, where there is no chat yet', () => {
    // `/agent/start` opens the session at `turn_count: 0` and `lead_turns >= 1`,
    // so this is the one answer the renderer can give with certainty.
    expect(leadWorkerChip(PAIR, false)).toEqual({
      model: 'claude-opus-5',
      provider: 'claude_code',
      role: 'lead',
    });
  });

  it('claims no role inside a chat, where the live half is not knowable', () => {
    // The turn count that decides it is daemon state this component is never
    // served. `{}` keeps the app-wide selection on the chip — the name it always
    // had — and drops the `(worker)` that was false for every chat's opening
    // turns.
    expect(leadWorkerChip(PAIR, true)).toEqual({});
  });

  it('says nothing at all when no pair is configured', () => {
    expect(leadWorkerChip(NO_PAIR, false)).toEqual({});
    expect(leadWorkerChip(NO_PAIR, true)).toEqual({});
  });

  /**
   * At `lead_turns: 0` the worker takes turn 1 (`turn_count < 0` never holds) and
   * the lead is reached only by the failure fallback, so the app-wide selection is
   * already the right name and `(lead)` would be false on Home too.
   */
  it('claims nothing at zero lead turns, where the worker takes turn 1', () => {
    expect(leadWorkerChip({ ...PAIR, leadTurns: 0 }, false)).toEqual({});
    expect(leadWorkerHandoverNote({ ...PAIR, leadTurns: 0 })).toBe(
      'Lead/worker mode. gpt-4.1-mini-2025-04-14 answers every turn; claude-opus-5 is the fallback model.'
    );
  });

  // The provider travels with the model or the chip hangs the worker provider's
  // tier, affiliation and disclosure on the lead's name.
  it('never returns a model without its provider', () => {
    for (const hasChat of [false, true]) {
      const chip = leadWorkerChip(PAIR, hasChat);
      expect(!!chip.model).toBe(!!chip.provider);
    }
  });
});

describe('leadWorkerActive', () => {
  it('is the lead model and nothing else', () => {
    expect(leadWorkerActive(PAIR)).toBe(true);
    expect(leadWorkerActive(NO_PAIR)).toBe(false);
    // A worker alone is just the app-wide selection, which every machine has.
    expect(leadWorkerActive({ ...NO_PAIR, workerModel: 'anything' })).toBe(false);
  });
});

describe('readLeadTurns', () => {
  it('takes a saved number', () => {
    expect(readLeadTurns(5)).toBe(5);
    expect(readLeadTurns('2')).toBe(2);
  });

  it('falls back to the daemon default for what a usize parse rejects', () => {
    // `/config/read` answers an unset key with `null`; a config file can hold
    // anything. Each of these is what `get_param::<usize>` would reject, so each
    // must resolve to what the daemon would then use.
    for (const raw of [null, undefined, '', 'three', {}, NaN, -4, 2.5]) {
      expect(readLeadTurns(raw)).toBe(DEFAULT_LEAD_TURNS);
    }
  });

  /**
   * ⚠ `0` is ACCEPTED by `get_param::<usize>`, so it must survive here. Treating
   * it as unusable and defaulting to 3 would make the chip claim `(lead)` for a
   * pair whose lead never answers the first turn — D7 with the halves swapped.
   * It is reachable through the shipped CLI: `configure.rs`'s `prompt_turns`
   * validator accepts any `u32::parse`, and `"0"` parses.
   */
  it('keeps a zero, which the daemon accepts', () => {
    expect(readLeadTurns(0)).toBe(0);
    expect(readLeadTurns('0')).toBe(0);
  });

  it('matches the Rust default', () => {
    // `DEFAULT_LEAD_TURNS` in crates/biorouter/src/providers/factory.rs.
    expect(DEFAULT_LEAD_TURNS).toBe(3);
  });
});

describe('leadWorkerHandoverNote', () => {
  it('states the handover, which is true of every surface', () => {
    expect(leadWorkerHandoverNote(PAIR)).toBe(
      'Lead/worker mode. claude-opus-5 answers the first 3 turns of a chat; gpt-4.1-mini-2025-04-14 takes the rest.'
    );
  });

  it('reads as English at one turn', () => {
    expect(leadWorkerHandoverNote({ ...PAIR, leadTurns: 1 })).toContain('the first turn of a chat');
  });

  it('names only what it knows when the worker key is unset', () => {
    expect(leadWorkerHandoverNote({ ...PAIR, workerModel: '' })).toBe(
      'Lead/worker mode. claude-opus-5 answers the first 3 turns of a chat.'
    );
  });

  it('is null with no pair, so nothing renders', () => {
    expect(leadWorkerHandoverNote(NO_PAIR)).toBeNull();
  });
});
