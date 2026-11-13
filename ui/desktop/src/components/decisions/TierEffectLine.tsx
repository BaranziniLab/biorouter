import { InfoTip } from '../ui/info-tip';
import { cn } from '../../utils';

/**
 * One short line for what a decision-settings change does to the chat's tier
 * (DR-33, design §5): "This chat will be public: joey-1 runs on Baseten.", with the
 * detail in an InfoTip beside it. The line is the consequence up front; the reason
 * and the way back are in the tip, so the dialog stays one sentence taller at most.
 */
export function TierEffectLine({
  line,
  info,
  infoLabel,
  testId,
  className,
}: {
  line: string;
  /** The tip, or `null` for a line that needs none. */
  info: string | null;
  /** What the tip is about, for its accessible name. */
  infoLabel: string;
  testId?: string;
  className?: string;
}) {
  return (
    <p
      className={cn('flex min-w-0 items-center gap-1 text-supporting text-text-default', className)}
      role="status"
      data-testid={testId}
    >
      <span className="min-w-0">{line}</span>
      {info ? <InfoTip label={infoLabel} help={info} /> : null}
    </p>
  );
}
