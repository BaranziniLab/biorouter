import { useCallback, useState, useSyncExternalStore } from 'react';
import { Check } from '../icons/app-icons';
import { Popover, PopoverContent, PopoverTrigger } from '../ui/popover';
import { Button } from '../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import {
  DEFAULT_REASONING_EFFORT,
  getReasoningEffort,
  REASONING_EFFORT_DESCRIPTIONS,
  REASONING_EFFORT_LABELS,
  REASONING_EFFORTS,
  ReasoningEffort,
  setReasoningEffort,
  subscribeToReasoningEffort,
} from '../../store/reasoningEffort';

/**
 * The effort glyph: three rising bars, filled up to the current level.
 *
 * It replaces a `Gauge`, and the reason is that a gauge is a picture of the
 * CONCEPT while this is a picture of the VALUE. The old glyph looked identical
 * at quick, normal and deep, so the control could only say "reasoning effort
 * lives here" and never "it is set to deep" — which is why the label had to be
 * spent on it whenever the setting was non-default. Bars carry the level
 * themselves, at a glance, in the width of an icon.
 *
 * ⚠ This is a FILLED glyph in a stroked icon set. That is deliberate and it is
 * the one place it is earned: `light()` pins strokeWidth 1.5 and `currentColor`
 * so that 115 files share one line weight, but a stroked bar chart cannot show
 * a filled-vs-empty distinction at 17px — the fill IS the data here, not
 * decoration. The unfilled bars stay `currentColor` at 25% rather than taking a
 * muted token, so the glyph still inherits its ink from whatever is around it
 * and survives all three families plus dark without a second declaration.
 */
const EFFORT_BAR_COUNT: Record<ReasoningEffort, number> = { quick: 1, normal: 2, deep: 3 };

// x, y and height per bar — rising left to right, on a 16px box. `rx` rounds the
// caps so the bars read as the same family as the app's rounded geometry.
const EFFORT_BARS = [
  { x: 1.5, y: 9, height: 5 },
  { x: 6.7, y: 6, height: 8 },
  { x: 11.9, y: 3, height: 11 },
];

function EffortBars({ effort, className }: { effort: ReasoningEffort; className?: string }) {
  const filled = EFFORT_BAR_COUNT[effort];
  return (
    <svg viewBox="0 0 16 16" fill="none" className={className} aria-hidden="true">
      {EFFORT_BARS.map((bar, index) => (
        <rect
          key={bar.x}
          x={bar.x}
          y={bar.y}
          width={2.6}
          height={bar.height}
          rx={1}
          fill="currentColor"
          opacity={index < filled ? 1 : 0.25}
        />
      ))}
    </svg>
  );
}

/**
 * BR-63: the composer's reasoning-effort control — the explore-vs-answer knob.
 *
 * The picked level rides on the next chat request (`reasoning_effort`), where it
 * maps to the provider's reasoning effort / thinking budget and to the agent
 * loop's exploration caps. `/effort quick|normal|deep` in the chat box does the
 * same thing for the whole session.
 */
export function BottomMenuReasoningEffort({ scope }: { scope: string }) {
  const [open, setOpen] = useState(false);
  const subscribe = useCallback(
    (listener: () => void) => subscribeToReasoningEffort(scope, listener),
    [scope]
  );
  const snapshot = useCallback(() => getReasoningEffort(scope), [scope]);
  const effort = useSyncExternalStore(subscribe, snapshot);
  const isDefault = effort === DEFAULT_REASONING_EFFORT;

  const select = (next: ReasoningEffort) => {
    setReasoningEffort(scope, next);
    setOpen(false);
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <button
              type="button"
              // `text-supporting`, not the `text-secondary` main.css prescribes
              // for a dense control — the override is explained once in
              // ChatInput.tsx, search "THE RAILS' TYPE".
              className="flex h-7 items-center gap-1.5 rounded-md px-0.5 cursor-pointer text-text-muted hover:bg-background-medium hover:text-text-default text-supporting"
              aria-label={`Reasoning effort: ${REASONING_EFFORT_LABELS[effort]}`}
            >
              <EffortBars effort={effort} className="size-[17px] shrink-0" />
              {/* The default stays the quiet state. It matters MORE now, not
                  less: the bars already say which of the three levels is set, so
                  spending composer width on a word that repeats the glyph is the
                  redundancy the new icon exists to remove. */}
              {!isDefault && <span>{REASONING_EFFORT_LABELS[effort]}</span>}
            </button>
          </PopoverTrigger>
        </TooltipTrigger>
        {/* Not the effort name again: the bars carry it, the word beside them
            carries it whenever it is not the default, and the aria-label
            carries it for a screen reader. The tooltip says what the control
            is FOR instead. */}
        <TooltipContent side="top">How hard to think on the next message</TooltipContent>
      </Tooltip>
      <PopoverContent side="top" align="center" className="w-64 p-0">
        <div className="border-b border-border-subtle px-3 py-2.5">
          <div className="text-label text-text-default">Reasoning effort</div>
          <div className="mt-0.5 text-supporting text-text-muted">
            Applies to this chat. Normal uses its /effort setting when set.
          </div>
        </div>
        <div className="p-1.5" role="menu" aria-label="Reasoning effort">
          {REASONING_EFFORTS.map((level) => {
            const selected = level === effort;
            return (
              <Button
                key={level}
                type="button"
                variant="ghost"
                size="sm"
                shape="pill"
                role="menuitemradio"
                aria-checked={selected}
                onClick={() => select(level)}
                className="h-auto w-full items-start justify-start gap-2 rounded-md px-2 py-1.5 text-left whitespace-normal hover:bg-background-medium/40"
              >
                <div className="min-w-0 flex-1">
                  <div className="font-medium text-text-default">
                    {REASONING_EFFORT_LABELS[level]}
                  </div>
                  <div className="text-supporting text-text-muted">
                    {REASONING_EFFORT_DESCRIPTIONS[level]}
                  </div>
                </div>
                <span aria-hidden="true" className="mt-0.5 size-3.5 shrink-0">
                  {selected && <Check className="size-3.5 text-text-default" />}
                </span>
              </Button>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}
