/**
 * The master privacy switch, as the renderer sees it (issue #56, DR-15).
 *
 * ⚠ **Its own module, and not `PrivacyPanel.tsx`.** These values are read by
 * `ConfigContext`, which every surface in the app mounts, by `PrivacyBadge`,
 * which is a leaf `ui/` component, and by the composer's off-state note
 * (`privacy/PrivacyTiersOffNote.tsx`). Leaving them in the panel
 * would pull the whole Settings → Privacy screen — switch, input, buttons — into
 * every session row and every model chip, and would make `ConfigContext` import
 * a settings screen that imports `ConfigContext`.
 */

/**
 * The config key that holds the master switch. One spelling, shared with the
 * daemon's `biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY`.
 */
export const PRIVACY_TIERS_KEY = 'BIOROUTER_PRIVACY_TIERS';

/**
 * The phrase the user must type to turn the feature off, byte-for-byte the
 * daemon's `PRIVACY_TIERS_DISABLE_PHRASE`. The daemon compares it EXACTLY, so a
 * panel that lower-cased or trimmed it would produce a 403 the user cannot
 * explain.
 */
export const DISABLE_PHRASE = 'DISABLE PRIVACY TIERS';

/**
 * `off` / `false` / `no` disable; anything else, including an absent key, is on.
 *
 * Mirrors `biorouter::privacy::privacy_tiers_value_is_on` — the daemon is the
 * authority and this is only what the renderer paints before the next read. The
 * `trim()` is part of that mirror, not a nicety: without the same surrounding
 * whitespace rule on both sides, a hand-edited `BIOROUTER_PRIVACY_TIERS: " off "`
 * renders as off in the panel while the daemon goes on enforcing, and the user
 * is told something false about a control they just used.
 */
export function privacyTiersEnabledFromConfig(value: unknown): boolean {
  if (typeof value === 'boolean') return value;
  if (typeof value !== 'string') return true;
  const v = value.trim().toLowerCase();
  return !(v === 'off' || v === 'false' || v === 'no');
}

/**
 * The daemon's report on the switch's RECORD, served beside the switch on the
 * same two config read paths (H3, 2026-09-10 security test drive): where the
 * record lives, and which door last wrote it. Mirrors
 * `biorouter::privacy::PRIVACY_TIERS_RECORD_KEY`.
 *
 * ⚠ **A report, never a setting.** The daemon composes it from what it loaded
 * and what its one confirmed write recorded, and both read paths overwrite any
 * copy of this key that `config.yaml` happens to hold — so nothing written
 * through `/config/upsert` reaches a reader, and nothing should try.
 */
export const PRIVACY_TIERS_RECORD_KEY = 'BIOROUTER_PRIVACY_TIERS_RECORD';

/**
 * Which door the record's value came through.
 *
 * - `settings` — Settings → Privacy's typed confirmation, unchanged since.
 * - `migration` — carried across from an older `config.yaml`, unchanged since.
 * - `unrecorded` — no door the app records wrote this value: the record was
 *   edited directly, or written by a Biorouter too old to stamp it.
 * - `default` — no readable record, so the fail-safe ON.
 */
export type PrivacyTiersOrigin = 'default' | 'settings' | 'migration' | 'unrecorded';

/** The last change a door recorded, as the record carries it. */
export type PrivacyTiersChange = {
  via: 'settings' | 'migration';
  /** The value that door wrote — which can differ from the record's now. */
  setTo: boolean;
  /** RFC 3339, UTC. */
  at: string;
  /** DR-20: the operating system confirmed the person at the keyboard. */
  systemAuthenticated: boolean;
  /** DR-16: the request carried the app's own proof of a user. */
  userAction: boolean;
};

export type PrivacyTiersRecord = {
  enabled: boolean;
  origin: PrivacyTiersOrigin;
  /** The record's absolute path, so a notice can say where to look. */
  path: string;
  lastChange: PrivacyTiersChange | null;
};

const ORIGINS: readonly PrivacyTiersOrigin[] = ['default', 'settings', 'migration', 'unrecorded'];

function isRecordObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function changeFromWire(value: unknown): PrivacyTiersChange | null {
  if (!isRecordObject(value)) return null;
  const { via, set_to, at, system_authenticated, user_action } = value;
  if ((via !== 'settings' && via !== 'migration') || typeof set_to !== 'boolean') return null;
  return {
    via,
    setTo: set_to,
    at: typeof at === 'string' ? at : '',
    systemAuthenticated: system_authenticated === true,
    userAction: user_action === true,
  };
}

/**
 * The report, or `null` when the daemon sent none — an older daemon behind the
 * external-backend setup, or a process that never loaded the switch.
 *
 * ⚠ **`null` must never hide the off-state.** Callers decide whether the tiers
 * are off from {@link privacyTiersEnabledFromConfig}; this only says HOW, and a
 * missing or malformed report degrades the explanation, not the notice.
 */
export function privacyTiersRecordFromConfig(value: unknown): PrivacyTiersRecord | null {
  if (!isRecordObject(value)) return null;
  const { enabled, origin, path, last_change } = value;
  if (typeof enabled !== 'boolean' || typeof path !== 'string') return null;
  if (!ORIGINS.includes(origin as PrivacyTiersOrigin)) return null;
  return {
    enabled,
    origin: origin as PrivacyTiersOrigin,
    path,
    lastChange: changeFromWire(last_change),
  };
}
