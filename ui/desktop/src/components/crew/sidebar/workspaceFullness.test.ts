import { describe, expect, it } from 'vitest';
import type { CrewWorkspaceUsage } from '../crewApi';
import { fullnessNeedsAttention, fullnessRose, workspaceFullness } from './workspaceFullness';

const MIB = 1024 * 1024;

/** The broker's standard budgets (`Quotas::STANDARD`), with `state` and `journal` in use. */
function usage(state: number, journal = 0, extra: Partial<CrewWorkspaceUsage> = {}) {
  return {
    state_bytes: state,
    state_limit: 16 * MIB,
    state_admin_headroom: MIB,
    journal_bytes: journal,
    journal_limit: 1024 * MIB,
    journal_admin_headroom: 16 * MIB,
    ...extra,
  };
}

describe('workspaceFullness (W2-UIW-20)', () => {
  it('measures against the space ordinary changes may use, not the whole limit', () => {
    // 12 MiB of the 15 MiB before the host's headroom: 80%, where 12 of 16 would read 75%.
    expect(workspaceFullness(usage(12 * MIB))).toEqual({ percent: 80, level: 'warn' });
    // Refused for every post, where 15 of 16 would read 93% and still not warn urgently.
    expect(workspaceFullness(usage(15 * MIB))).toEqual({ percent: 100, level: 'full' });
  });

  it('warns at 80%, urgently at 95%, and says full only when posting stops', () => {
    const room = 15 * MIB;
    expect(workspaceFullness(usage(Math.floor(room * 0.79)))?.level).toBe('ok');
    expect(workspaceFullness(usage(Math.ceil(room * 0.8)))?.level).toBe('warn');
    expect(workspaceFullness(usage(Math.ceil(room * 0.95)))?.level).toBe('urgent');
    // One byte short is never "100% full".
    expect(workspaceFullness(usage(room - 1))).toEqual({ percent: 99, level: 'urgent' });
    expect(workspaceFullness(usage(room + 1))).toEqual({ percent: 100, level: 'full' });
  });

  it('reads the fuller of the state and the journal', () => {
    // A small state and a journal at 90% of its 1008 MiB.
    const fullness = workspaceFullness(usage(MIB, Math.ceil(1008 * MIB * 0.9)));
    expect(fullness).toEqual({ percent: 90, level: 'warn' });
  });

  it('treats a missing headroom as none, as an older usage shape would', () => {
    const bare = {
      state_bytes: 13 * MIB,
      state_limit: 16 * MIB,
      journal_bytes: 0,
      journal_limit: 1,
    };
    expect(workspaceFullness(bare)).toEqual({ percent: 81, level: 'warn' });
  });

  it('says nothing without usage or with budgets that make no sense', () => {
    expect(workspaceFullness(undefined)).toBeNull();
    expect(workspaceFullness(null)).toBeNull();
    expect(
      workspaceFullness(
        usage(MIB, 0, { state_admin_headroom: 16 * MIB, journal_admin_headroom: 1024 * MIB })
      )
    ).toBeNull();
    // Attachments alone never warn: a full attachment space only refuses attachments.
    expect(
      workspaceFullness(usage(0, 0, { attachment_bytes: 100, attachment_limit: 100 }))?.level
    ).toBe('ok');
  });

  it('needs attention from 80%, and rises only into a more pressing level', () => {
    const ok = workspaceFullness(usage(MIB));
    const warn = workspaceFullness(usage(13 * MIB));
    const urgent = workspaceFullness(usage(Math.ceil(15 * MIB * 0.96)));
    const full = workspaceFullness(usage(16 * MIB));
    expect(fullnessNeedsAttention(ok)).toBe(false);
    expect(fullnessNeedsAttention(warn)).toBe(true);
    expect(fullnessNeedsAttention(null)).toBe(false);
    expect(fullnessRose(ok, warn)).toBe(true);
    expect(fullnessRose(warn, urgent)).toBe(true);
    expect(fullnessRose(urgent, full)).toBe(true);
    expect(fullnessRose(warn, warn)).toBe(false);
    expect(fullnessRose(full, warn)).toBe(false);
    // The first reading is the baseline.
    expect(fullnessRose(null, full)).toBe(false);
  });
});
