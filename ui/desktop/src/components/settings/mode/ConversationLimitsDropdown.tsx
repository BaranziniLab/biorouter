import { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronDown } from '../../icons/app-icons';
import { Input } from '../../ui/input';

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

interface ConversationLimitsDropdownProps {
  /**
   * The saved `BIOROUTER_MAX_TURNS` exactly as stored, or `null` when none is
   * saved. A stored value that is not a limit (0, a negative number) is shown as
   * it is, so the person can see it and fix it.
   */
  maxTurns: number | null;
  onMaxTurnsChange: (value: number) => void;
}

export const ConversationLimitsDropdown = ({
  maxTurns,
  onMaxTurnsChange,
}: ConversationLimitsDropdownProps) => {
  const [isExpanded, setIsExpanded] = useState(false);
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

  const flushSave = useCallback(() => {
    if (saveTimer.current !== null) {
      clearTimeout(saveTimer.current);
      saveTimer.current = null;
    }
    const value = pendingSave.current;
    pendingSave.current = null;
    if (value !== null) {
      ownSaves.current.push(value);
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

  const toggleExpanded = () => {
    setIsExpanded(!isExpanded);
  };

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

  /**
   * TWO ROWS, as siblings — not a row plus a boxed panel inside a wrapper.
   *
   * The wrapper was doing two kinds of damage. The trailing hairline is
   * suppressed by `.biorouter-settings-row:last-child`, which is relative to a
   * row's own PARENT: inside a wrapper the disclosure's row could never be the
   * list's last child, so the Mode section ended on a hairline with nothing
   * under it. And the panel it wrapped was a `rounded-element
   * bg-background-medium/55` card — a filled, rounded ground on a tab whose
   * whole rhythm is hairline-separated rows with no fill of their own.
   *
   * As siblings, `:last-child` lands correctly in both states with no extra
   * rule: collapsed, the trigger is last and drops its hairline; expanded, the
   * trigger keeps it (it is now a separator) and Max turns drops its own.
   *
   * The cost is the max-height/opacity collapse, which needs the panel mounted
   * to animate. A mount-time fade is the honest replacement — `animate-in
   * fade-in` is already the app's idiom for content that arrives — and it is the
   * right trade: a hairline in the wrong place is a defect, an expansion that
   * does not slide is a preference.
   */
  return (
    <>
      <button
        onClick={toggleExpanded}
        aria-expanded={isExpanded}
        className="biorouter-settings-row group flex w-full items-center justify-between px-3 py-2.5"
      >
        <h3 className="text-label text-text-default">Chat limits</h3>

        <ChevronDown
          className={`h-4 w-4 text-text-muted transition-transform duration-200 ease-in-out ${
            isExpanded ? 'rotate-180' : 'rotate-0'
          }`}
        />
      </button>

      {isExpanded && (
        <div className="biorouter-settings-row flex min-w-0 animate-in items-center justify-between gap-3 px-3 py-2.5 fade-in duration-100">
          <div className="min-w-0 flex-1">
            <h4 className="text-label text-text-default" id="max-turns-label">
              Max turns
            </h4>
            <p className="mt-0.5 max-w-md text-supporting text-text-muted">
              Maximum agent turns before Biorouter asks for user input
            </p>
            {message && (
              <p
                className="mt-1 max-w-md text-supporting text-text-danger"
                id="max-turns-problem"
                role="alert"
              >
                {message}
              </p>
            )}
          </div>
          <Input
            type="number"
            min="1"
            step="1"
            value={draft}
            aria-labelledby="max-turns-label"
            aria-invalid={message ? true : undefined}
            aria-describedby={message ? 'max-turns-problem' : undefined}
            onChange={(e) => handleChange(e.target.value)}
            onBlur={flushSave}
            className="w-20"
          />
        </div>
      )}
    </>
  );
};
