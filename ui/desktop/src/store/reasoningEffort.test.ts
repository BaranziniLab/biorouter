import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  adoptDraftReasoningEffort,
  draftReasoningScope,
  getReasoningEffort,
  reasoningEffortForRequest,
  resetReasoningEffortForTests,
  sessionReasoningScope,
  setReasoningEffort,
  subscribeToReasoningEffort,
} from './reasoningEffort';

const a = sessionReasoningScope('a');
const b = sessionReasoningScope('b');
const storageKey = (scope: string) => `biorouter.reasoningEffort.v2:${scope}`;

describe('conversation reasoning effort', () => {
  beforeEach(() => {
    localStorage.clear();
    sessionStorage.clear();
    resetReasoningEffortForTests();
  });

  it('starts every unknown chat on Normal without adopting the old global setting', () => {
    localStorage.setItem('biorouter.reasoningEffort', 'deep');
    expect(getReasoningEffort(a)).toBe('normal');
    setReasoningEffort(a, 'quick');
    expect(getReasoningEffort(b)).toBe('normal');
  });

  it('omits Normal so the session /effort setting still applies', () => {
    expect(reasoningEffortForRequest(getReasoningEffort(a))).toBeUndefined();
    setReasoningEffort(a, 'deep');
    expect(reasoningEffortForRequest(getReasoningEffort(a))).toBe('deep');
    setReasoningEffort(a, 'normal');
    expect(reasoningEffortForRequest(getReasoningEffort(a))).toBeUndefined();
  });

  it('restores distinct choices after a renderer reload', () => {
    setReasoningEffort(a, 'quick');
    setReasoningEffort(b, 'deep');
    resetReasoningEffortForTests();
    expect(getReasoningEffort(a)).toBe('quick');
    expect(getReasoningEffort(b)).toBe('deep');
  });

  it('ignores invalid persisted choices', () => {
    localStorage.setItem(storageKey(a), 'ludicrous');
    expect(reasoningEffortForRequest(getReasoningEffort(a))).toBeUndefined();
  });

  it('notifies only the affected chat and only on a real change', () => {
    const listener = vi.fn();
    const unsubscribe = subscribeToReasoningEffort(a, listener);
    setReasoningEffort(b, 'deep');
    expect(listener).not.toHaveBeenCalled();
    setReasoningEffort(a, 'quick');
    setReasoningEffort(a, 'quick');
    expect(listener).toHaveBeenCalledTimes(1);
    unsubscribe();
    setReasoningEffort(a, 'deep');
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it('receives another window’s same-session changes without changing other sessions', () => {
    setReasoningEffort(a, 'quick');
    setReasoningEffort(b, 'quick');
    const listener = vi.fn();
    subscribeToReasoningEffort(a, listener);
    localStorage.setItem(storageKey(a), 'deep');
    const changed = new StorageEvent('storage', { key: storageKey(a), newValue: 'deep' });
    Object.defineProperty(changed, 'storageArea', { value: localStorage });
    window.dispatchEvent(changed);
    expect(getReasoningEffort(a)).toBe('deep');
    expect(getReasoningEffort(b)).toBe('quick');
    expect(listener).toHaveBeenCalledOnce();
    localStorage.clear();
    const cleared = new StorageEvent('storage', { key: null });
    Object.defineProperty(cleared, 'storageArea', { value: localStorage });
    window.dispatchEvent(cleared);
    expect(getReasoningEffort(a)).toBe('normal');
  });

  it('keeps drafts in window storage and transfers only the submitted draft', () => {
    const draftA = draftReasoningScope('tab:a');
    const draftB = draftReasoningScope('tab:b');
    setReasoningEffort(draftA, 'quick');
    setReasoningEffort(draftB, 'deep');
    expect(localStorage.getItem(storageKey(draftA))).toBeNull();
    resetReasoningEffortForTests();
    expect(getReasoningEffort(draftA)).toBe('quick');
    adoptDraftReasoningEffort(draftA, 'a', getReasoningEffort(draftA));
    expect(reasoningEffortForRequest(getReasoningEffort(a))).toBe('quick');
    expect(getReasoningEffort(draftA)).toBe('normal');
    expect(getReasoningEffort(draftB)).toBe('deep');
  });

  it('preserves a newer draft choice while an earlier session creation completes', () => {
    const draft = draftReasoningScope('home');
    setReasoningEffort(draft, 'quick');
    const submitted = getReasoningEffort(draft);
    setReasoningEffort(draft, 'deep');
    adoptDraftReasoningEffort(draft, 'a', submitted);
    expect(getReasoningEffort(a)).toBe('quick');
    expect(getReasoningEffort(draft)).toBe('deep');
  });
});
