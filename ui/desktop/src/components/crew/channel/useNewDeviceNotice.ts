import { useCallback, useEffect, useMemo, useState } from 'react';
import type { CrewDevice } from '../crewApi';

/**
 * The devices on the viewer's own account that this computer has already seen, per connection and
 * person, in this viewer's `localStorage`. It is a convenience, not a security control: storage can
 * be empty, blocked or cleared, and every access is guarded so the page renders without it.
 */
const STORAGE_PREFIX = 'biorouter.crew.seenDevices.v1';

export function seenDevicesKey(connectionId: string, actorId: string): string {
  return `${STORAGE_PREFIX}:${connectionId}:${actorId}`;
}

/** The fingerprints recorded for this key, or `null` when nothing (readable) was recorded. */
export function readSeenDevices(key: string): string[] | null {
  try {
    const raw = window.localStorage.getItem(key);
    if (raw === null) return null;
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed.filter((item): item is string => typeof item === 'string')
      : null;
  } catch {
    return null;
  }
}

export function writeSeenDevices(key: string, fingerprints: readonly string[]): void {
  try {
    window.localStorage.setItem(key, JSON.stringify([...new Set(fingerprints)]));
  } catch {
    // Storage is a convenience here; without it the notice simply is not remembered.
  }
}

function usableDevices(devices: readonly CrewDevice[] | null | undefined): CrewDevice[] {
  return Array.isArray(devices)
    ? devices.filter(
        (device): device is CrewDevice =>
          Boolean(device) && typeof device.fingerprint === 'string' && device.fingerprint !== ''
      )
    : [];
}

export interface NewDeviceNotice {
  /** Unix seconds of the newest unseen device, or null when the broker did not say. */
  addedAt: number | null;
}

/**
 * "A new device was added to your account" (ui-redesign-spec, "Where errors render: exactly once").
 *
 * Compares the verified snapshot's `actor.devices` with the list this computer last acknowledged.
 * The first time a person is seen here the current list is recorded silently — every device is
 * "new" to a computer that has never looked, and flagging them would teach people to ignore the
 * notice. After that, any fingerprint not in the record raises the notice until it is acknowledged.
 * Pass `devices: null` (no verified snapshot) to show nothing.
 */
export function useNewDeviceNotice(
  connectionId: string,
  actorId: string | null,
  devices: readonly CrewDevice[] | null | undefined
): { notice: NewDeviceNotice | null; acknowledge(): void } {
  const key = actorId && connectionId ? seenDevicesKey(connectionId, actorId) : null;
  const current = useMemo(() => usableDevices(devices), [devices]);
  const fingerprints = useMemo(() => current.map((device) => device.fingerprint), [current]);
  const [seen, setSeen] = useState<string[] | null>(null);
  const [seenKey, setSeenKey] = useState<string | null>(null);

  useEffect(() => {
    if (!key || devices === null || devices === undefined) return;
    const stored = readSeenDevices(key);
    if (stored === null) {
      writeSeenDevices(key, fingerprints);
      setSeen(fingerprints);
    } else {
      setSeen(stored);
    }
    setSeenKey(key);
  }, [key, devices, fingerprints]);

  const acknowledge = useCallback(() => {
    if (!key) return;
    writeSeenDevices(key, fingerprints);
    setSeen(fingerprints);
    setSeenKey(key);
  }, [key, fingerprints]);

  if (!key || seenKey !== key || seen === null) return { notice: null, acknowledge };
  const unseen = current.filter((device) => !seen.includes(device.fingerprint));
  if (unseen.length === 0) return { notice: null, acknowledge };
  const dates = unseen
    .map((device) => device.added_at)
    .filter((value): value is number => typeof value === 'number' && Number.isFinite(value));
  return { notice: { addedAt: dates.length ? Math.max(...dates) : null }, acknowledge };
}
