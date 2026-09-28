import { isBrowserSurface } from '../../utils/surface';

/**
 * What a browser-served session is told instead of a 409.
 *
 * `docs/deployment/serve-decisions.md` **SD-1**: a browser session runs whatever
 * provider and model the machine was configured with, because the daemon behind
 * `biorouter serve` holds no proof-of-user mechanism (SD-7) and every write to a
 * capability config key is refused by issue #56's DR-16 guard. SD-1's own
 * consequence clause is what this file implements — *"the interface must explain
 * the refusal rather than appear broken. A disabled picker with a reason is the
 * requirement; a 409 toast is not."*
 *
 * ⚠ **These strings are for a human.** The daemon's refusal body
 * (`crates/biorouter/src/privacy/refusal.rs`) is addressed to an AI agent and
 * tells the reader to open the desktop application — correct for its audience,
 * and useless to someone in a browser who has no desktop application to open.
 * That file is privacy-critical and pinned by repo-grep tests; nothing here
 * changes it, and none of this should be copied back into it.
 *
 * ⚠ **One definition, seven surfaces.** The onboarding gate, the composer's
 * model chip, the switch-model dialog, Settings > Models, the reset control, the
 * provider grid, the lead/worker dialog and the config editor all render these
 * same sentences. A second copy of a sentence is a sentence that goes stale in
 * one of its homes and stays wrong there.
 */

/** The command that changes the model, on the machine running the daemon. */
export const HOST_CONFIGURE_COMMAND = 'biorouter configure';

/** The command whose host owns the choice. */
export const HOST_SERVE_COMMAND = 'biorouter serve';

/** Heading for a control that is inert because the host owns the choice. */
export const HOST_MANAGED_MODEL_TITLE = 'The model is set on the host';

/** One line, for a chip or a row with no room for the reason. */
export const HOST_MANAGED_MODEL_SHORT = `Biorouter is open in a browser, so the model comes from the machine running ${HOST_SERVE_COMMAND}.`;

/** The full explanation, for anywhere with room for three sentences. */
export const HOST_MANAGED_MODEL_REASON =
  `Biorouter is open in a browser, so the model comes from the machine running ${HOST_SERVE_COMMAND}. ` +
  `Change it there with ${HOST_CONFIGURE_COMMAND}, then reload this page. A browser tab cannot ` +
  `switch models, and that is what keeps a private conversation from silently moving to a public model.`;

/** Heading for the first-run screen when the host has configured nothing yet. */
export const HOST_MANAGED_ONBOARDING_TITLE = 'Choose a model on the host';

/**
 * The first-run screen's body.
 *
 * SD-1's dead-end case: a fresh browser user lands in onboarding, which is
 * exactly where provider selection happens, and every card there ends in a write
 * that 409s. So the cards are not offered at all — the user is told what to run
 * on the host instead.
 */
export const HOST_MANAGED_ONBOARDING_REASON =
  `No provider is configured yet, and a browser tab cannot set one up. On the machine running ` +
  `${HOST_SERVE_COMMAND}, run ${HOST_CONFIGURE_COMMAND} to choose a provider and model, then ` +
  `reload this page. Fixing that choice on the host is what keeps a private conversation from ` +
  `silently moving to a public model.`;

/**
 * W2-PRV-2, round 4. The settings that decide where a provider sends its
 * requests, and the key they carry: hosts, endpoints, regions, the AWS profile
 * and endpoint overrides, certificate paths, the coding-agent commands. The
 * daemon changes one only for a request that proves a person asked, and a
 * browser never can, so there they belong to the host as the model does
 * (`../settings/destinationConfigKeys.ts` names them).
 */
export const HOST_MANAGED_DESTINATION_SHORT = `Biorouter is open in a browser, so where a provider sends its key is set on the computer running ${HOST_SERVE_COMMAND}.`;

/** The full explanation, for a provider form. */
export const HOST_MANAGED_DESTINATION_REASON =
  `Biorouter is open in a browser, so where a provider sends its requests and its key is set on ` +
  `the computer running ${HOST_SERVE_COMMAND}. Change it there with ${HOST_CONFIGURE_COMMAND}, or ` +
  `in the Biorouter desktop app on that computer, then reload this page. A browser tab cannot ` +
  `change it, because nothing here can confirm that a person made the change.`;

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

/**
 * The reason a browser-served control is disabled, or `null` on the desktop.
 *
 * `null` rather than `''` is deliberate: a caller spreading this into a `title`
 * wants the attribute absent on the desktop, not present and empty.
 */
export function hostManagedModelReason(): string | null {
  return isBrowserSurface() ? HOST_MANAGED_MODEL_REASON : null;
}
