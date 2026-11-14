/**
 * What a browser-served session is told when the host keeps the provider and model choice.
 *
 * `docs/deployment/serve-decisions.md` **SD-1** made that every browser: the daemon behind
 * `biorouter serve` held no proof-of-user mechanism (SD-7), and every write to a capability
 * config key was refused by issue #56's DR-16 guard. **SD-18** opened it: the daemon now admits
 * the served page itself to the provider routes, and says so through `GET /config/provider_setup`
 * (`utils/providerSetupAccess.ts`). So these sentences are for the browser the daemon has NOT
 * admitted (it refused, it is an older daemon, or it has not answered yet), and none of them may
 * claim a browser can never set up a provider. SD-1's consequence clause still holds for that
 * case: *"the interface must explain the refusal rather than appear broken. A disabled picker
 * with a reason is the requirement; a 409 toast is not."*
 *
 * ⚠ **These strings are for a human.** The daemon's refusal body
 * (`crates/biorouter/src/privacy/refusal.rs`) is addressed to an AI agent and
 * tells the reader to open the desktop application — correct for its audience,
 * and useless to someone in a browser who has no desktop application to open.
 * That file is privacy-critical and pinned by repo-grep tests; nothing here
 * changes it, and none of this should be copied back into it.
 *
 * ⚠ **Words only.** This file imports nothing that runs: the copy is read by modules a test
 * bundles on its own (`steerUnavailableCopy.ts` takes `HOST_SERVE_COMMAND` from here), and a
 * logic import here would drag the API client into each of them. The one function that pairs
 * these words with the daemon's answer, `hostManagedModelReason`, lives in
 * `utils/providerSetupAccess.ts`.
 *
 * ⚠ **One definition, seven surfaces.** The onboarding gate, the composer's
 * model chip, the switch-model dialog, Settings > Models, the reset control, the
 * provider grid and the config editor all render these
 * same sentences. A second copy of a sentence is a sentence that goes stale in
 * one of its homes and stays wrong there.
 */

/** The command that changes the model, on the machine running the daemon. */
export const HOST_CONFIGURE_COMMAND = 'biorouter configure';

/** The command whose host owns the choice. */
export const HOST_SERVE_COMMAND = 'biorouter serve';

/** Heading for a control that is inert because the host owns the choice. */
export const HOST_MANAGED_MODEL_TITLE = 'The model is set on the host';

/** One line, for a chip or a row with no room for the reason, and the note's visible line. */
export const HOST_MANAGED_MODEL_SHORT = `Biorouter is open in a browser, so the model comes from the machine running ${HOST_SERVE_COMMAND}.`;

/**
 * What to do, and why the choice is the host's here: the info icon beside
 * {@link HOST_MANAGED_MODEL_SHORT} (owner, 2026-11-13: one line up front, the detail on hover).
 * Two sentences. Shown only where the daemon did not admit this browser (SD-18).
 */
export const HOST_MANAGED_MODEL_HELP =
  `Change it there with ${HOST_CONFIGURE_COMMAND}, then reload this page. Keeping the choice on ` +
  `the host keeps a private conversation from silently moving to a public model.`;

/** The full explanation as one string, for a disabled control's description. */
export const HOST_MANAGED_MODEL_REASON = `${HOST_MANAGED_MODEL_SHORT} ${HOST_MANAGED_MODEL_HELP}`;

/** Heading for the first-run screen when the host has configured nothing yet. */
export const HOST_MANAGED_ONBOARDING_TITLE = 'Choose a model on the host';

/**
 * The first-run screen's one visible line, when the daemon gave no sentence of its own
 * (`providerSetupRefusal()`): an older daemon, or a read that failed. The daemon's sentence
 * replaces it whenever there is one, since it knows why it refused.
 */
/** The panel's one visible line when the daemon gave no sentence of its own. */
export const HOST_MANAGED_ONBOARDING_REASON = `No provider is set up yet. Choose one on the computer running ${HOST_SERVE_COMMAND}.`;

/** The first-run panel's InfoTip: what to do there, and why the choice stays on the host. */
export const HOST_MANAGED_ONBOARDING_HELP =
  `Run ${HOST_CONFIGURE_COMMAND} on that computer, then reload this page. Keeping the choice ` +
  `there keeps a private conversation from silently moving to a public model.`;

/**
 * Where a provider's key is kept, said on the first-run page of a browser the daemon admitted
 * (SD-18) in place of the desktop's keychain sentence: the key is written by the daemon, on the
 * computer running `biorouter serve`, and this tab keeps no copy.
 */
export const HOST_KEY_STORAGE_NOTICE = `Keys are kept on the computer running ${HOST_SERVE_COMMAND}, not in this browser.`;

/**
 * W2-PRV-2, round 4. The settings that decide where a provider sends its
 * requests, and the key they carry: hosts, endpoints, regions, the AWS profile
 * and endpoint overrides, certificate paths, the coding-agent commands. The
 * daemon changes one only for a request that proves a person asked, which a
 * browser does only once the daemon admits its page (SD-18), so until then they
 * belong to the host as the model does
 * (`../settings/destinationConfigKeys.ts` names them).
 */
export const HOST_MANAGED_DESTINATION_SHORT = `Biorouter is open in a browser, so where a provider sends its key and requests is set on the computer running ${HOST_SERVE_COMMAND}.`;

/**
 * What to do, and why this page cannot: the info icon beside
 * {@link HOST_MANAGED_DESTINATION_SHORT}. Two sentences.
 */
export const HOST_MANAGED_DESTINATION_HELP =
  `Change it there with ${HOST_CONFIGURE_COMMAND} or the desktop app, then reload this page. This ` +
  `page cannot change it, because that computer has not let this browser set up providers.`;

/** The full explanation as one string, for a provider form. */
export const HOST_MANAGED_DESTINATION_REASON = `${HOST_MANAGED_DESTINATION_SHORT} ${HOST_MANAGED_DESTINATION_HELP}`;

/** The error beside a required destination setting the host has not set. */
export function hostManagedDestinationRequired(fieldName: string): string {
  return `${fieldName} is set on the computer running ${HOST_SERVE_COMMAND}, with ${HOST_CONFIGURE_COMMAND}.`;
}

/**
 * Beside a custom provider's URL. The daemon lets a browser move the URL only
 * when the key is typed again, since a kept key would follow the URL.
 */
export const HOST_MANAGED_CUSTOM_URL =
  `Biorouter is open in a browser, so moving this provider to a new URL needs its key typed ` +
  `again. Otherwise change the URL in the Biorouter desktop app on the computer running ` +
  `${HOST_SERVE_COMMAND}.`;
