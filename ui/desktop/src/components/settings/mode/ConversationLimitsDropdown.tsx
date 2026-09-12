import { useState } from 'react';
import { ChevronDown } from '../../icons/app-icons';
import { Input } from '../../ui/input';

interface ConversationLimitsDropdownProps {
  maxTurns: number;
  onMaxTurnsChange: (value: number) => void;
}

export const ConversationLimitsDropdown = ({
  maxTurns,
  onMaxTurnsChange,
}: ConversationLimitsDropdownProps) => {
  const [isExpanded, setIsExpanded] = useState(false);

  const toggleExpanded = () => {
    setIsExpanded(!isExpanded);
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
            <h4 className="text-label text-text-default">Max turns</h4>
            <p className="mt-0.5 max-w-md text-supporting text-text-muted">
              Maximum agent turns before Biorouter asks for user input
            </p>
          </div>
          <Input
            type="number"
            min="1"
            max="10000"
            value={maxTurns}
            onChange={(e) => onMaxTurnsChange(Number(e.target.value))}
            className="w-20"
          />
        </div>
      )}
    </>
  );
};
