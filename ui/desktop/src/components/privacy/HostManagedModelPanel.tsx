import { CopyField } from '../ui/copy-field';
import { InfoTip } from '../ui/info-tip';
import {
  HOST_CONFIGURE_COMMAND,
  HOST_MANAGED_ONBOARDING_HELP,
  HOST_MANAGED_ONBOARDING_STEP,
  HOST_MANAGED_ONBOARDING_TITLE,
} from './hostManagedModelCopy';

/**
 * The first-run panel a browser user sees in place of the provider cards
 * (SD-1's dead-end case; see `hostManagedModelCopy.ts`).
 *
 * ⚠ **Unlike `HostManagedModelNote` this renders whatever the surface.** Its one
 * call site — `ProviderGuard` — has already chosen this branch over the card
 * stack, and a component that went quiet here would leave a welcome header above
 * an empty page, which is a worse dead end than the 409 it replaces.
 *
 * The command is the shared `CopyField`, like every other command on first run:
 * the person has to paste it into a terminal on the host (ruling 25, a named
 * exception to the presentation-only rule for frozen privacy UI; the text shown
 * is unchanged).
 *
 * One step up front, the reason on hover (owner, 2026-11-13): the visible line
 * is what to run and that the page reloads after it; why it has to happen on
 * the host is the info icon beside the title, which sits outside the heading so
 * the heading's name stays the title alone.
 */
export function HostManagedModelPanel() {
  return (
    <div data-testid="host-managed-model-panel" className="br-first-run-card br-fill-surface p-5">
      <div className="flex min-w-0 items-center gap-1">
        <h2 className="text-label text-text-default">{HOST_MANAGED_ONBOARDING_TITLE}</h2>
        <InfoTip label="choosing a model on the host" help={HOST_MANAGED_ONBOARDING_HELP} />
      </div>
      <p className="mt-2 text-body text-text-muted">{HOST_MANAGED_ONBOARDING_STEP}</p>
      <CopyField className="mt-4" value={HOST_CONFIGURE_COMMAND} label={HOST_CONFIGURE_COMMAND} />
    </div>
  );
}
