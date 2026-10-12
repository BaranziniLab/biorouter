import { useEffect, useRef } from 'react';
import { useConfig } from '../ConfigContext';
import { recall, rememberIfUnknown, type LastKnownKey } from './lastKnown';
import {
  APPROVAL_MODE_KEY,
  MAX_TURNS_KEY,
  approvalModeFromConfig,
  maxTurnsFromConfig,
} from './mode/storedSettings';
import { ANNOUNCE_ONLY_KEY } from './app/WorkspaceSettingsSection';

type ReadConfig = (key: string, isSecret: boolean) => Promise<unknown>;

/**
 * Ask for every setting the Chat and App tabs show before either tab is opened (vision round 8,
 * V4-R7-2).
 *
 * Settings opens on Models, and the first visit to Chat or App is the one a person makes within a
 * second or two. Those tabs start each control from the last answer this window heard
 * (`useLastKnown`), but on that first visit there is none, so the approval select was an empty box,
 * Max turns an empty field and the switches a column that filled in two steps. These reads run
 * while Settings opens, so the answer is waiting.
 *
 * Read-only, and never louder than the tabs themselves: a failed read is dropped (the tab reads
 * again and says what it always said), and an answer is kept only if nothing newer has landed, so
 * a switch the person flipped while one was in flight is not set back.
 */
export function warmSettings(read: ReadConfig): void {
  const ask = (key: LastKnownKey, get: () => Promise<unknown>) => {
    if (recall(key) !== undefined) return;
    // `then` turns a synchronous throw (a main-process bridge that is not there) into a rejection.
    void Promise.resolve()
      .then(get)
      .then(
        (value) => rememberIfUnknown(key, value),
        () => {}
      );
  };

  ask('approvalMode', async () => approvalModeFromConfig(await read(APPROVAL_MODE_KEY, false)));
  ask('maxTurns', async () => maxTurnsFromConfig(await read(MAX_TURNS_KEY, false)));
  ask('announceOnly', async () => (await read(ANNOUNCE_ONLY_KEY, false)) === true);
  ask('menuBarIcon', () => window.electron.getMenuBarIconState());
  ask('wakelock', () => window.electron.getWakelockState());
  ask('spellcheck', () => window.electron.getSpellcheckState());
  // Only macOS has a Dock, and only there does the App tab show the row.
  ask('dockIcon', async () => {
    if (window.electron.platform !== 'darwin') return undefined;
    return window.electron.getDockIconState();
  });
}

/** Run {@link warmSettings} once when Settings mounts. */
export function useWarmSettings(): void {
  const { read } = useConfig();
  const readRef = useRef(read);
  readRef.current = read;
  useEffect(() => {
    warmSettings((key, isSecret) => readRef.current(key, isSecret));
  }, []);
}
