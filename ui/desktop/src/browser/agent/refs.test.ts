import { describe, expect, it } from 'vitest';
import { RefStore, fallbackMatch, normalizeRef } from './refs';

/**
 * BB-12: references. A node keeps its reference across snapshots, numbers are never reused, a
 * navigation invalidates them with a reason, and the role and name fallback fails closed on
 * ambiguity.
 */

const node = (
  backendNodeId: number,
  extra: Partial<{ frameKey: string; role: string; name: string; unique: boolean }> = {}
) => ({
  backendNodeId,
  frameKey: extra.frameKey ?? 'main',
  role: extra.role ?? 'button',
  name: extra.name ?? `Button ${backendNodeId}`,
  unique: extra.unique ?? true,
});

describe('RefStore', () => {
  it('gives a node the same reference in every snapshot', () => {
    const refs = new RefStore();
    const first = refs.assign(node(10));
    const second = refs.assign(node(11));
    expect(first).toBe('@e1');
    expect(second).toBe('@e2');
    expect(refs.assign(node(10))).toBe('@e1');
    expect(refs.get('@e1')?.backendNodeId).toBe(10);
    expect(refs.get('e2')?.backendNodeId).toBe(11);
  });

  it('refreshes what the latest snapshot said about a node', () => {
    const refs = new RefStore();
    refs.assign(node(10, { name: 'Save', unique: true }));
    refs.assign(node(10, { name: 'Save draft', unique: false }));
    expect(refs.get('@e1')).toMatchObject({ name: 'Save draft', unique: false });
  });

  it('never reuses a number, even after navigation', () => {
    const refs = new RefStore();
    refs.assign(node(1));
    refs.assign(node(2));
    refs.invalidateAll('https://site-a.test/next');
    // The same backend node id in the new document is a new node with a new number.
    expect(refs.assign(node(1))).toBe('@e3');
    expect(refs.lastNumber).toBe(3);
    expect(refs.get('@e1')).toBeNull();
  });

  it('says why an old reference fails and what to do', () => {
    const refs = new RefStore();
    refs.assign(node(1));
    refs.invalidateAll('https://site-a.test/a');
    expect(refs.staleReason('@e1', 'https://site-a.test/b')).toBe(
      '@e1 is from an earlier page; the tab is now at https://site-a.test/b. Take a new browser_snapshot.'
    );
    expect(refs.staleReason('@e99', 'https://site-a.test/b')).toContain(
      'is not a reference from this tab'
    );
  });

  it('invalidates only the frame that navigated', () => {
    const refs = new RefStore();
    refs.assign(node(1));
    refs.assign(node(2, { frameKey: 'frame-1' }));
    expect(refs.invalidateFrame('frame-1', 'https://site-a.test/')).toBe(true);
    expect(refs.get('@e1')).not.toBeNull();
    expect(refs.get('@e2')).toBeNull();
    expect(refs.staleReason('@e2', 'https://site-a.test/')).toContain(
      'in a frame that has since navigated'
    );
  });

  it('says a discarded tab was reloaded', () => {
    const refs = new RefStore();
    refs.assign(node(1));
    refs.invalidateAll('', 'discarded');
    expect(refs.staleReason('@e1', '')).toContain('reloaded');
  });

  it('normalizes the spellings of a reference', () => {
    expect(normalizeRef(' e5 ')).toBe('@e5');
    expect(normalizeRef('@e5')).toBe('@e5');
  });
});

describe('fallbackMatch', () => {
  const target = {
    ref: '@e5',
    backendNodeId: 1,
    frameKey: 'main',
    role: 'button',
    name: 'Save',
    unique: true,
  };

  it('moves a unique reference to the one node with its role and name', () => {
    expect(
      fallbackMatch(target, [
        { backendNodeId: 7, role: 'button', name: 'Save' },
        { backendNodeId: 8, role: 'link', name: 'Save' },
      ])
    ).toEqual({ ok: true, backendNodeId: 7 });
  });

  it('fails closed when several nodes match', () => {
    const result = fallbackMatch(target, [
      { backendNodeId: 7, role: 'button', name: 'Save' },
      { backendNodeId: 8, role: 'button', name: 'Save' },
      { backendNodeId: 9, role: 'button', name: 'Save' },
    ]);
    expect(result).toEqual({
      ok: false,
      reason:
        '@e5 no longer exists and 3 elements now match [button] "Save". Take a new browser_snapshot.',
    });
  });

  it('never moves a reference that was not unique when it was taken', () => {
    const result = fallbackMatch({ ...target, unique: false }, [
      { backendNodeId: 7, role: 'button', name: 'Save' },
    ]);
    expect(result.ok).toBe(false);
  });

  it('fails when nothing matches', () => {
    expect(fallbackMatch(target, []).ok).toBe(false);
  });
});
