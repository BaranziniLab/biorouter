import { useCallback, useEffect, useRef, useState } from 'react';
import { Input } from '../../ui/input';
import { SettingRow } from '../../ui/setting-row';
import { approvalsCopy } from '../chat/copy';

/**
 * The agent's own default, `biorouter::agents::DEFAULT_MAX_TURNS`, shown when
 * nothing is saved. This field used to show 1000 while the agent used 100.
 * `ConversationLimitsDropdown.test.tsx` reads the Rust constant so the two
 * cannot drift again.
 */
export const DEFAULT_MAX_TURNS = 100;

/** The largest value the daemon's `u32` setting can hold. */
const MAX_TURNS_CEILING = 4_294_967_295;

/**
 * How long the field waits after the last keystroke before it saves. Long
 * enough that deleting "100" one digit at a time saves nothing on the way.
 */
export const MAX_TURNS_SAVE_DELAY_MS = 400;

/**
 * A max-turns entry as a limit, or `null` when it is not one.
 *
 * Only a whole number of at least 1 is a limit. An empty field is NOT 0:
 * `Number('')` is 0, and saving it made every new chat stop before its first
 * model call. A negative number was saved too, and silently ignored.
 */
export function parseMaxTurns(text: string): number | null {
  const trimmed = text.trim();
  if (!/^\d+$/.test(trimmed)) {
    return null;
  }
  const value = Number(trimmed);
  return value >= 1 && value <= MAX_TURNS_CEILING ? value : null;
}

interface MaxTurnsRowProps {
  /**
   * The saved `BIOROUTER_MAX_TURNS` exactly as stored, or `null` when none is
   * saved. A stored value that is not a limit (0, a negative number) is shown as
   * it is, so the person can see it and fix it.
   */
  maxTurns: number | null;
  onMaxTurnsChange: (value: number) => void;
}

/**
 * Settings > Chat > Approvals > Max turns. Always shown: it used to sit behind a "Chat limits"
 * disclosure that held exactly this one field (spec §3.13).
 */
export const MaxTurnsRow = ({ maxTurns, onMaxTurnsChange }: MaxTurnsRowProps) => {
  const shown = maxTurns ?? DEFAULT_MAX_TURNS;
  const [draft, setDraft] = useState(String(shown));

  /**
   * T3-SH-8. Saves wait for the typing to settle, and a save's own answer never
   * rewrites the field.
   *
   * Saving on every keystroke meant that deleting "100" saved 10, then 1, and
   * each save came back, as a new `maxTurns`, after the field was already
   * empty. The effect below then wrote that number into the field: the person
   * saw no message, and a value they had just deleted. Now nothing is saved
   * until the entry has been a limit for {@link MAX_TURNS_SAVE_DELAY_MS}, a
   * value this field saved is recognised when it comes back (`ownSaves`), and a
   * save still waiting when the field goes away is made then rather than lost.
   */
  const pendingSave = useRef<number | null>(null);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const ownSaves = useRef<number[]>([]);
  const onMaxTurnsChangeRef = useRef(onMaxTurnsChange);
  onMaxTurnsChangeRef.current = onMaxTurnsChange;
  const shownRef = useRef(shown);
  shownRef.current = shown;

  const flushSave = useCallback(() => {
    if (saveTimer.current !== null) {
      clearTimeout(saveTimer.current);
      saveTimer.current = null;
    }
    const value = pendingSave.current;
    pendingSave.current = null;
    if (value !== null) {
      // A save of the value already shown changes nothing, so it never comes
      // back as a change; recorded, it would swallow a later real one.
      if (value !== shownRef.current) ownSaves.current.push(value);
      onMaxTurnsChangeRef.current(value);
    }
  }, []);
  useEffect(() => flushSave, [flushSave]);

  // Follow the stored value when it arrives (the read is async) or changes
  // elsewhere, but never overwrite an entry that already means that value, and
  // never overwrite anything with the echo of this field's own save.
  useEffect(() => {
    const own = ownSaves.current.indexOf(shown);
    if (own !== -1) {
      ownSaves.current.splice(0, own + 1);
      return;
    }
    setDraft((current) => (parseMaxTurns(current) === shown ? current : String(shown)));
  }, [shown]);

  const draftValue = parseMaxTurns(draft);
  const storedIsInvalid = maxTurns !== null && parseMaxTurns(String(maxTurns)) === null;
  // What is wrong with the field, if anything. A stored value that is not a
  // limit is named as the saved value, so the person knows it came from
  // before and was not their typing.
  const message =
    draftValue !== null
      ? null
      : storedIsInvalid && draft === String(maxTurns)
        ? `The saved value, ${maxTurns}, is not a whole number of at least 1. Enter a new value.`
        : 'Enter a whole number of at least 1.';

  const handleChange = (text: string) => {
    setDraft(text);
    // An entry that is not a limit cancels a save still waiting: the last
    // thing typed is the one that counts.
    if (saveTimer.current !== null) clearTimeout(saveTimer.current);
    saveTimer.current = null;
    pendingSave.current = parseMaxTurns(text);
    if (pendingSave.current !== null) {
      saveTimer.current = setTimeout(flushSave, MAX_TURNS_SAVE_DELAY_MS);
    }
  };

  // The problem line is the row's status, not help: it is an error, so it stays visible
  // (principle 2) and is announced (`role="alert"`). `SettingRow` adds it to the field's
  // `aria-describedby` beside the InfoTip's help.
  return (
    <SettingRow
      label={approvalsCopy.maxTurns}
      help={approvalsCopy.maxTurnsHelp}
      status={
        message ? (
          <span className="text-text-danger" id="max-turns-problem" role="alert">
            {message}
          </span>
        ) : undefined
      }
    >
      <Input
        type="number"
        min="1"
        step="1"
        value={draft}
        aria-invalid={message ? true : undefined}
        onChange={(e) => handleChange(e.target.value)}
        onBlur={flushSave}
        className="w-20"
      />
    </SettingRow>
  );
};
