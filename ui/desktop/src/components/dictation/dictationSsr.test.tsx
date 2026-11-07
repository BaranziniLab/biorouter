import { useRef } from 'react';
import { renderToString } from 'react-dom/server';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DictationStatus } from './dictationApi';
import { DictationButton, DictationNotices } from './DictationButton';
import {
  getDictationStatusSnapshot,
  refreshDictationStatus,
  resetDictationStoreForTests,
  useDictationStatus,
} from './dictationStore';
import { fakeStatusApi, speechStatus } from './dictationTestKit';
import { markPrivateDraft, resetPrivateDraftsForTests, usePrivateDraft } from './privateDraft';
import { useDictationComposer } from './useDictationComposer';

/**
 * Every dictation store gives React a server snapshot. A transcript rendered with
 * `renderToString` (the geometry tests under `components/transcript/`) holds a message editor,
 * and so a microphone, and React's server renderer throws "Missing getServerSnapshot" for a
 * `useSyncExternalStore` without one (frontend.yml run 42378067316, after PR-19).
 */
const KEY = 'tab:ssr';

function Composer() {
  const ref = useRef<HTMLTextAreaElement>(null);
  const control = useDictationComposer({
    key: KEY,
    textAreaRef: ref,
    storesDraft: false,
    target: () => ({ target_kind: 'chat', session_id: 'chat-ssr' }),
    setByState: () => {},
    isEmpty: () => true,
  });
  const flagged = usePrivateDraft(KEY);
  const status = useDictationStatus();
  return (
    <div data-gate={control.gate} data-private={String(flagged)} data-load={status.load}>
      <DictationNotices dictationKey={KEY} />
      <textarea ref={ref} />
      <DictationButton control={control} setView={() => {}} />
    </div>
  );
}

describe('a server render of a composer with a microphone', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    resetDictationStoreForTests();
    resetPrivateDraftsForTests();
  });

  it('completes, with no console error', () => {
    const errors = vi.spyOn(console, 'error').mockImplementation(() => {});
    const html = renderToString(<Composer />);
    expect(errors).not.toHaveBeenCalled();
    expect(html).toContain('data-gate="free"');
    expect(html).toContain('data-private="false"');
    expect(html).toContain('data-load="unloaded"');
  });

  it('reads the private flag as the window holds it', () => {
    markPrivateDraft(KEY);
    expect(renderToString(<Composer />)).toContain('data-private="true"');
  });

  it('sees the status store as it starts, whatever the daemon answered', async () => {
    const status: DictationStatus = speechStatus();
    resetDictationStoreForTests(fakeStatusApi(() => status).api);
    await refreshDictationStatus();
    expect(getDictationStatusSnapshot().load).toBe('loaded');
    expect(renderToString(<Composer />)).toContain('data-load="unloaded"');
  });
});
