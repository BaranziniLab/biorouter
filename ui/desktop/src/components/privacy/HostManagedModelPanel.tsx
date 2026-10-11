import {
  HOST_CONFIGURE_COMMAND,
  HOST_MANAGED_ONBOARDING_REASON,
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
 */
export function HostManagedModelPanel() {
  return (
    <div data-testid="host-managed-model-panel" className="br-first-run-card br-fill-surface p-5">
      <h2 className="text-label text-text-default">{HOST_MANAGED_ONBOARDING_TITLE}</h2>
      <p className="mt-2 text-body text-text-muted">{HOST_MANAGED_ONBOARDING_REASON}</p>
      <pre className="mt-4 overflow-x-auto rounded-element bg-background-default px-3 py-2 text-supporting text-text-default">
        <code>{HOST_CONFIGURE_COMMAND}</code>
      </pre>
      <p className="mt-3 text-supporting text-text-muted">
        Reload this page once that command has finished.
      </p>
    </div>
  );
}
