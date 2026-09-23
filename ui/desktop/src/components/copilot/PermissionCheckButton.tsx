import { Check, Loader2 } from '../icons/app-icons';
import { Button } from '../ui/button';
import type { RuntimeVerdict } from './CopilotSetup';

/**
 * The result sentence. Never colour alone: the icon and the words both carry the
 * meaning, so the outcome survives a monochrome or colour-blind reading.
 */
const RESULT: Record<RuntimeVerdict, string> = {
  ready: 'All OS permissions are allowed.',
  blocked: 'A required OS permission is missing — see the detail above.',
  unavailable: 'The native runtime is unavailable — see the repair steps above.',
  unverified: 'The backend could not confirm OS permissions — see the detail above.',
};

/**
 * The permission re-check control, shared by the in-chat panel and Settings so
 * the two cannot drift.
 *
 * It exists because the bare button was indistinguishable from a dead one: the
 * probe it runs usually returns exactly what is already on screen, so a
 * sub-second label flip was the entire feedback. The pending state and the
 * explicit result line are the change — they report that the check RAN, which
 * the unchanged detail above it cannot.
 *
 * `verdict` is the verdict of the runtime CURRENTLY on screen, not a verdict
 * captured when the check ran. The chat panel re-polls status every 2s, so a
 * cached sentence could sit under a detail that contradicts it — reading "All OS
 * permissions are allowed." directly beneath "Accessibility: not allowed" after
 * someone revoked it in System Settings. `checked` only records that a check
 * happened, which is what makes the button feel answered.
 */
export function PermissionCheckButton({
  label,
  checking,
  checked,
  verdict,
  onCheck,
}: {
  label: string;
  checking: boolean;
  checked: boolean;
  verdict?: RuntimeVerdict;
  onCheck: () => void;
}) {
  const showResult = !checking && checked && verdict !== undefined;
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Button
        size="sm"
        variant="outline"
        disabled={checking}
        aria-busy={checking}
        onClick={onCheck}
      >
        {checking && <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />}
        {checking ? 'Checking…' : label}
      </Button>
      {/* Announced on arrival: the visible detail above may be byte-identical to
          what was already there, so the confirmation is the only signal that the
          click did anything. */}
      <span role="status" aria-live="polite" className="min-w-0">
        {showResult && (
          <span
            className={
              verdict === 'ready'
                ? 'flex items-center gap-1.5 text-text-success biorouter-check-settled'
                : 'flex items-center gap-1.5 text-text-muted'
            }
          >
            {verdict === 'ready' && <Check className="size-3.5 shrink-0" aria-hidden="true" />}
            {RESULT[verdict]}
          </span>
        )}
      </span>
    </div>
  );
}
