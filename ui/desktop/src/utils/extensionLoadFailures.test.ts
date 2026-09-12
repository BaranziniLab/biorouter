import { describe, it, expect, beforeEach } from 'vitest';
import {
  dismissExtensionLoadFailure,
  getExtensionLoadFailures,
  markExtensionLoadFailuresAnnounced,
  recordExtensionLoadResults,
  resetExtensionLoadFailuresForTests,
  subscribeExtensionLoadFailures,
} from './extensionLoadFailures';

describe('the standing extension-failure record', () => {
  beforeEach(() => {
    resetExtensionLoadFailuresForTests();
  });

  it('survives a renderer reload — the in-memory cache is not the record', () => {
    recordExtensionLoadResults([{ name: 'cdwagent', success: false, error: 'spawn ENOENT' }]);
    markExtensionLoadFailuresAnnounced(['cdwagent']);

    // A reload drops every module-level cache but not localStorage.
    resetInMemoryCacheOnly();

    const [failure] = getExtensionLoadFailures();
    expect(failure.name).toBe('cdwagent');
    expect(failure.announced).toBe(true);
  });

  it('answers "nothing new" for a failure already announced', () => {
    recordExtensionLoadResults([{ name: 'cdwagent', success: false, error: 'spawn ENOENT' }]);
    markExtensionLoadFailuresAnnounced(['cdwagent']);

    expect(
      recordExtensionLoadResults([{ name: 'cdwagent', success: false, error: 'spawn ENOENT' }])
    ).toHaveLength(0);
  });

  // A different failure is different news, even from the same extension.
  it('treats a CHANGED error as new', () => {
    recordExtensionLoadResults([{ name: 'cdwagent', success: false, error: 'spawn ENOENT' }]);
    markExtensionLoadFailuresAnnounced(['cdwagent']);

    const fresh = recordExtensionLoadResults([
      { name: 'cdwagent', success: false, error: 'connection refused' },
    ]);
    expect(fresh.map((f) => f.name)).toEqual(['cdwagent']);
  });

  it('drops the record when the extension later loads cleanly', () => {
    recordExtensionLoadResults([{ name: 'cdwagent', success: false, error: 'spawn ENOENT' }]);
    recordExtensionLoadResults([{ name: 'cdwagent', success: true }]);

    expect(getExtensionLoadFailures()).toHaveLength(0);
  });

  it('does not consume the announcement when nothing was rendered', () => {
    const fresh = recordExtensionLoadResults([
      { name: 'cdwagent', success: false, error: 'spawn ENOENT' },
    ]);
    expect(fresh).toHaveLength(1);
    // No `markExtensionLoadFailuresAnnounced` — the toast never rendered.
    expect(
      recordExtensionLoadResults([{ name: 'cdwagent', success: false, error: 'spawn ENOENT' }])
    ).toHaveLength(1);
  });

  it('notifies subscribers on record and on dismissal', () => {
    const seen: number[] = [];
    const unsubscribe = subscribeExtensionLoadFailures((failures) => seen.push(failures.length));

    recordExtensionLoadResults([{ name: 'cdwagent', success: false, error: 'spawn ENOENT' }]);
    dismissExtensionLoadFailure('cdwagent');
    unsubscribe();
    recordExtensionLoadResults([{ name: 'medcp', success: false, error: 'spawn ENOENT' }]);

    expect(seen).toEqual([1, 0]);
  });

  it('reads an unparseable record as an empty one instead of throwing', () => {
    localStorage.setItem('biorouter.extensions.loadFailures', '{not json');
    resetInMemoryCacheOnly();
    expect(getExtensionLoadFailures()).toEqual([]);
  });
});

/**
 * Simulate a renderer load: the module's in-memory cache is gone, localStorage
 * is not. `resetExtensionLoadFailuresForTests` clears BOTH, so it cannot stand
 * in for this — and the distinction is the entire point of the module.
 */
function resetInMemoryCacheOnly(): void {
  const saved = localStorage.getItem('biorouter.extensions.loadFailures');
  resetExtensionLoadFailuresForTests();
  if (saved !== null) {
    localStorage.setItem('biorouter.extensions.loadFailures', saved);
  }
}
