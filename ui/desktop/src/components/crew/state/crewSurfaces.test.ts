import { describe, expect, it } from 'vitest';
import { resolveErrorSlot } from './crewActions';
import { isSnapshotBoundDialog, nextUiAfterReset } from './crewSurfaces';
import { DRAFT_CLEARING_OBSERVATION_CODES, observationFailureOutcome } from './observationFailure';
import type { CrewUi, DialogIntent, ErrorSource } from './types';

const pane = { mode: 'agent' } as const;

describe('surface resets', () => {
  it('closes snapshot-bound dialogs on refresh and keeps the pane', () => {
    const ui: CrewUi = { dialog: { kind: 'edit-profile' }, pane };
    expect(nextUiAfterReset(ui, 'refresh')).toEqual({ dialog: null, pane });
  });

  it.each<DialogIntent>([
    { kind: 'join' },
    { kind: 'host' },
    { kind: 'connection-settings', connectionId: 'conn-1' },
    { kind: 'keys' },
    { kind: 'invite-people' },
    { kind: 'let-in', username: 'bob' },
    { kind: 'confirm', confirm: { action: 'remove-connection', connectionId: 'conn-1' } },
  ])('keeps %o open through a refresh', (dialog) => {
    const ui: CrewUi = { dialog, pane: null };
    expect(nextUiAfterReset(ui, 'refresh')).toBe(ui);
    expect(isSnapshotBoundDialog(dialog)).toBe(false);
  });

  it.each<DialogIntent>([
    { kind: 'workspace-settings', tab: 'people' },
    { kind: 'workspace-settings', tab: 'privacy' },
    { kind: 'create-team' },
    { kind: 'create-channel', teamId: 'team-1' },
    { kind: 'add-people', target: 'channel', targetId: 'channel-1' },
    { kind: 'transfer-ownership', channelId: 'channel-1' },
    { kind: 'rename', target: 'team', targetId: 'team-1' },
    { kind: 'edit-profile' },
    { kind: 'share-path' },
    { kind: 'confirm', confirm: { action: 'archive-channel', channelId: 'channel-1' } },
  ])('treats %o as bound to the verified snapshot', (dialog) => {
    expect(isSnapshotBoundDialog(dialog)).toBe(true);
  });

  it.each([
    'protected-cleared',
    'channel-changed',
    'channel-revoked',
    'connection-changed',
  ] as const)(
    'closes the pane and snapshot-bound dialogs on %s, keeping connection dialogs',
    (reason) => {
      expect(nextUiAfterReset({ dialog: { kind: 'create-team' }, pane }, reason)).toEqual({
        dialog: null,
        pane: null,
      });
      expect(nextUiAfterReset({ dialog: { kind: 'join' }, pane }, reason)).toEqual({
        dialog: { kind: 'join' },
        pane: null,
      });
    }
  );

  it('closes the dialog after a finished mutation and leaves a started task to the layout', () => {
    const ui: CrewUi = { dialog: { kind: 'create-team' }, pane };
    expect(nextUiAfterReset(ui, 'mutated')).toEqual({ dialog: null, pane });
    expect(nextUiAfterReset(ui, 'run-started')).toBe(ui);
  });
});

describe('error slots', () => {
  const error = (source: ErrorSource) => ({ message: 'failed', source });
  const mounted = (...sources: ErrorSource[]) =>
    new Map<ErrorSource, number>(sources.map((source) => [source, 1]));

  it('renders an error at its surface while that surface is mounted', () => {
    expect(resolveErrorSlot(error('pane:agent'), mounted('pane:agent', 'composer'))).toBe(
      'pane:agent'
    );
    expect(resolveErrorSlot(error('dialog:create-team'), mounted('dialog:create-team'))).toBe(
      'dialog:create-team'
    );
  });

  it('falls back to the connection bar when the surface is gone', () => {
    expect(resolveErrorSlot(error('pane:agent'), mounted('composer'))).toBe('global');
    expect(resolveErrorSlot(error('composer'), new Map([['composer', 0]]))).toBe('global');
  });

  it('always renders observer and global errors in the connection bar', () => {
    expect(resolveErrorSlot(error('observer'), mounted('observer'))).toBe('observer');
    expect(resolveErrorSlot(error('global'), mounted())).toBe('global');
    expect(resolveErrorSlot(null, mounted('composer'))).toBeNull();
  });
});

describe('observation failures', () => {
  it.each(DRAFT_CLEARING_OBSERVATION_CODES)('clears the draft for %s', (code) => {
    const outcome = observationFailureOutcome('Access changed.', code);
    expect(outcome.clearDraft).toBe(true);
    expect(outcome.text).toMatch(/^Access changed\. Access or privacy changed/);
  });

  it.each([undefined, 'observation_refused', 'stale_cursor', 'temporary'])(
    'keeps the draft for %s',
    (code) => {
      const outcome = observationFailureOutcome('Updates stopped.', code);
      expect(outcome.clearDraft).toBe(false);
      expect(outcome.text).toContain('unsent draft is retained');
    }
  );
});
