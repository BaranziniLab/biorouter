import type { PrivacyTiersRecord } from '../settings/privacy/privacyTiers';

/**
 * What the app says while privacy tiers are off, and how the switch got there
 * (H3, 2026-09-10 security test drive).
 *
 * ⚠ **One definition, two surfaces** — the note above the composer and the
 * strip in Settings → Privacy. The note's one control opens that strip, so a
 * second hand-written account there would be the first thing to contradict
 * the sentence that sent the user to it. Each surface keeps its own statement
 * of the consequence; the headline, the HOW and the path are shared.
 *
 * ⚠ **What the origin can and cannot vouch for.** The record is an ordinary
 * file DR-17 leaves writable by anything holding `developer__shell`, so the
 * stamp that makes an origin `settings` can be forged by something that knows
 * its shape. What it reliably catches is the write the drive measured — an
 * overwrite with `{"enabled": false}` — and a one-field flip of a stamped
 * record, because the stamp names the value it wrote. So the copy never says
 * "you turned this off": it says what the record says, and the unrecorded case
 * is the one that gets the stronger tone.
 */
export type PrivacyTiersOffCopy = {
  /** `danger` only when no door the app records wrote the OFF. */
  tone: 'warning' | 'danger';
  headline: string;
  /** How the switch got to off, or `null` when the daemon did not say. */
  how: string | null;
  /** The record's path, or `null` when the daemon sent no report. */
  path: string | null;
};

/** The composer note's statement of the consequence. */
export const PRIVACY_TIERS_OFF_CONSEQUENCE =
  'Nothing is separating private chats, extensions or knowledge bases from public models.';

/** The OFF headline in every case but one: the state first, the explanation after. */
const OFF = 'Privacy tiers are off.';

function formatWhen(at: string | undefined): string | null {
  if (!at) return null;
  const date = new Date(at);
  if (Number.isNaN(date.getTime())) return null;
  return date.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

const on = (when: string | null) => (when ? ` on ${when}` : '');

export function privacyTiersOffCopy(record: PrivacyTiersRecord | null): PrivacyTiersOffCopy {
  // No report, or one describing an ON switch while the switch itself reads
  // off: both come from one config snapshot, so this is a daemon that serves
  // the switch but not the record. Say what is known and nothing more.
  if (!record || record.enabled) {
    return { tone: 'warning', headline: OFF, how: null, path: null };
  }

  const change = record.lastChange;
  const when = formatWhen(change?.at);

  switch (record.origin) {
    case 'settings':
      return {
        tone: 'warning',
        headline: OFF,
        how:
          `They were turned off in Settings → Privacy${on(when)}` +
          `${change?.systemAuthenticated ? ', and your operating system confirmed it' : ''}.`,
        path: record.path,
      };
    case 'migration':
      return {
        tone: 'warning',
        headline: OFF,
        how: `They were carried over as off from an older version’s config.yaml${on(when)}.`,
        path: record.path,
      };
    case 'unrecorded':
      return {
        tone: 'danger',
        headline: 'Privacy tiers are off, and they were turned off outside the app.',
        how: change?.setTo
          ? `The last change recorded in the app turned them on${on(when)}, so their record ` +
            'has been edited since.'
          : 'No change in Settings → Privacy is recorded for them: their record was edited ' +
            'directly — by hand, by a script or by an agent with a shell — or written by an ' +
            'older version of Biorouter.',
        path: record.path,
      };
    default:
      // `default` is the fail-safe ON and cannot describe an OFF switch; if the
      // two ever disagree, explain nothing rather than guess.
      return { tone: 'warning', headline: OFF, how: null, path: record.path };
  }
}
