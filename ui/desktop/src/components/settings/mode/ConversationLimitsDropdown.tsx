import { useCallback, useEffect, useRef, useState } from 'react';
import { Input } from '../../ui/input';
import { SegmentedControl } from '../../ui/segmented-control';
import { SettingRow } from '../../ui/setting-row';
import { approvalsCopy } from '../chat/copy';

/**
 * The agent's own default, `biorouter::agents::DEFAULT_MAX_TURNS`, shown when
 * nothing is saved. Zero means Unlimited.
 * `ConversationLimitsDropdown.test.tsx` reads the Rust constant so the two
 * cannot drift again.
 */
export const DEFAULT_MAX_TURNS = 0;

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
 * Zero explicitly selects Unlimited. An empty field is not zero, so clearing
 * a saved limit never silently removes it.
 */
export function parseMaxTurns(text: string): number | null {
  const trimmed = text.trim();
  if (!/^\d+$/.test(trimmed)) {
    return null;
  }
  const value = Number(trimmed);
  return value >= 0 && value <= MAX_TURNS_CEILING ? value : null;
}

interface MaxTurnsRowProps {
  /**
   * The saved `BIOROUTER_MAX_TURNS` exactly as stored, or `null` when none is
   * saved. A stored value that is invalid (such as a negative number) is shown as
   * it is, so the person can see it and fix it.
   */
  maxTurns: number | null;
  onMaxTurnsChange: (value: number) => void | Promise<void>;
  /**
   * False until the saved value has been read. Both modes stay unselected and disabled rather
   * than presenting a guessed default. Defaults to true when the caller already holds the value.
   */
  loaded?: boolean;
}

interface MaxTurnsControlsProps {
  id?: string;
  'aria-labelledby'?: string;
  'aria-describedby'?: string;
  disabled?: boolean;
  limited: boolean;
  draft: string;
  invalid: boolean;
  onModeChange: (mode: string) => void;
  onDraftChange: (text: string) => void;
  onBlur: () => void;
}

function MaxTurnsControls({
  id,
  'aria-labelledby': labelledBy,
  'aria-describedby': describedBy,
  disabled,
  limited,
  draft,
  invalid,
  onModeChange,
  onDraftChange,
  onBlur,
}: MaxTurnsControlsProps) {
  return (
    <div
      className="br-max-turns-controls"
      aria-busy={disabled || undefined}
      onBlur={(event) => {
        // Moving to Unlimited must cancel the finite draft before anything saves it.
        if (!event.currentTarget.contains(event.relatedTarget)) onBlur();
      }}
    >
      <SegmentedControl
        id={id}
        aria-labelledby={labelledBy}
        aria-describedby={describedBy}
        options={[
          { value: 'unlimited', label: 'Unlimited' },
          { value: 'limited', label: 'Set a limit' },
        ]}
        value={disabled ? '' : limited ? 'limited' : 'unlimited'}
        disabled={disabled}
        onValueChange={onModeChange}
      />
      {limited && !disabled ? (
        <div className="br-max-turns-input">
          <label htmlFor={`${id}-turns`} className="text-secondary text-text-default">
            Turns
          </label>
          <Input
            id={`${id}-turns`}
            type="number"
            min="1"
            max={MAX_TURNS_CEILING}
            step="1"
            value={draft}
            aria-describedby={describedBy}
            aria-invalid={invalid || undefined}
            onChange={(event) => onDraftChange(event.target.value)}
            className="h-control-sm w-28"
          />
        </div>
      ) : null}
    </div>
  );
}

/**
 * Settings > Chat > Approvals > Max turns. Always shown: it used to sit behind a "Chat limits"
 * disclosure that held exactly this one field (spec §3.13).
 */
export const MaxTurnsRow = ({ maxTurns, onMaxTurnsChange, loaded = true }: MaxTurnsRowProps) => {
  const shown = maxTurns ?? DEFAULT_MAX_TURNS;
  const [limited, setLimited] = useState(shown !== 0);
  const [draft, setDraft] = useState(shown === 0 ? '' : String(shown));
  const lastPositiveDraft = useRef(
    shown > 0 && parseMaxTurns(String(shown)) !== null ? String(shown) : ''
  );

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
  const ownSaves = useRef<Array<{ value: number; completed: boolean }>>([]);
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
      const ownSave = { value, completed: false };
      const alreadyShown = value === shownRef.current;
      const retire = () => {
        ownSaves.current = ownSaves.current.filter((save) => save !== ownSave);
      };
      // An earlier queued write can change the prop before even a same-value save completes.
      ownSaves.current.push(ownSave);
      const saved = onMaxTurnsChangeRef.current(value);
      if (saved) {
        saved.then(() => {
          ownSave.completed = true;
          // A successful same-value write has no prop-change echo to consume its token.
          if (value === shownRef.current) {
            const own = ownSaves.current.indexOf(ownSave);
            if (own !== -1) ownSaves.current.splice(0, own + 1);
          }
        }, retire);
      } else {
        ownSave.completed = true;
        // Synchronous callers have no completion to await and no changed value to echo.
        if (alreadyShown) retire();
      }
    }
  }, []);
  useEffect(() => flushSave, [flushSave]);

  // Follow the stored value when it arrives (the read is async) or changes
  // elsewhere, but never overwrite an entry that already means that value, and
  // never overwrite anything with the echo of this field's own save.
  useEffect(() => {
    // React may coalesce completed writes, while a later duplicate is still pending.
    let own = ownSaves.current.reduce(
      (last, save, index) => (save.value === shown && save.completed ? index : last),
      -1
    );
    if (own === -1) own = ownSaves.current.findIndex((save) => save.value === shown);
    if (own !== -1) {
      // Writes are ordered by ModeSection. A later acknowledgement supersedes older failed
      // or coalesced saves, which must not mask a future change from another window.
      ownSaves.current.splice(0, own + 1);
      return;
    }
    if (saveTimer.current !== null) clearTimeout(saveTimer.current);
    saveTimer.current = null;
    pendingSave.current = null;
    setLimited(shown !== 0);
    if (shown !== 0) {
      setDraft((current) => (parseMaxTurns(current) === shown ? current : String(shown)));
      if (shown > 0 && parseMaxTurns(String(shown)) !== null) {
        lastPositiveDraft.current = String(shown);
      }
    }
  }, [shown, loaded]);

  const draftValue = parseMaxTurns(draft);
  const storedIsInvalid = maxTurns !== null && parseMaxTurns(String(maxTurns)) === null;
  // What is wrong with the field, if anything. A stored value that is not a
  // limit is named as the saved value, so the person knows it came from
  // before and was not their typing.
  const message =
    !loaded || !limited || (draftValue !== null && draftValue > 0)
      ? null
      : storedIsInvalid && draft === String(maxTurns)
        ? `The saved value, ${maxTurns}, is not a positive whole number. Enter a new value.`
        : 'Enter a whole number from 1 to 4,294,967,295.';

  const handleChange = (text: string) => {
    setDraft(text);
    // An entry that is not a limit cancels a save still waiting: the last
    // thing typed is the one that counts.
    if (saveTimer.current !== null) clearTimeout(saveTimer.current);
    saveTimer.current = null;
    const value = parseMaxTurns(text);
    pendingSave.current = value !== null && value > 0 ? value : null;
    if (pendingSave.current !== null) {
      lastPositiveDraft.current = text;
      saveTimer.current = setTimeout(flushSave, MAX_TURNS_SAVE_DELAY_MS);
    }
  };

  const handleModeChange = (mode: string) => {
    setLimited(mode === 'limited');
    if (mode === 'limited') {
      handleChange(lastPositiveDraft.current);
    } else {
      if (saveTimer.current !== null) clearTimeout(saveTimer.current);
      saveTimer.current = null;
      pendingSave.current = 0;
      flushSave();
    }
  };

  // The problem line is the row's status, not help: it is an error, so it stays visible
  // (principle 2) and is announced (`role="alert"`). `SettingRow` adds it to the field's
  // `aria-describedby` beside the InfoTip's help.
  return (
    <SettingRow
      label={approvalsCopy.maxTurns}
      help={approvalsCopy.maxTurnsHelp}
      className="br-max-turns-row"
      disabled={!loaded}
      status={
        message ? (
          <span className="text-text-danger" id="max-turns-problem" role="alert">
            {message}
          </span>
        ) : undefined
      }
    >
      <MaxTurnsControls
        limited={limited}
        draft={draft}
        invalid={message !== null}
        onModeChange={handleModeChange}
        onDraftChange={handleChange}
        onBlur={flushSave}
      />
    </SettingRow>
  );
};
