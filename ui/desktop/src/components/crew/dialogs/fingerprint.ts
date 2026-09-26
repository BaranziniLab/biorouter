import { useEffect, useState } from 'react';

/**
 * The workspace key fingerprint, as `hello` reports it and as the Join dialog shows it: SHA-256 of
 * the 32 key bytes, lowercase hex (`biorouter_crew::workspace_key_fingerprint`), and its short form
 * for the eye, the first 16 hex digits upper-cased in groups of four (`grouped_fingerprint`).
 *
 * Computed here, from the pinned key the connection already holds, so Connection settings can show
 * the same string the host compares at join time. It is display only: nothing is verified by it.
 */

const HEX_KEY = /^[0-9a-f]{64}$/i;

export async function workspaceKeyFingerprint(keyHex: string): Promise<string | null> {
  if (!HEX_KEY.test(keyHex) || typeof crypto === 'undefined' || !crypto.subtle) return null;
  const bytes = new Uint8Array(32);
  for (let index = 0; index < 32; index += 1) {
    bytes[index] = Number.parseInt(keyHex.slice(index * 2, index * 2 + 2), 16);
  }
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, '0')).join('');
}

/** `3f2a9c1e…` → `3F2A 9C1E 77B0 D4E1`. */
export function groupedFingerprint(fingerprintHex: string): string {
  const digits = Array.from(fingerprintHex)
    .filter((char) => /[0-9a-f]/i.test(char))
    .slice(0, 16)
    .map((char) => char.toUpperCase());
  const groups: string[] = [];
  for (let index = 0; index < digits.length; index += 4) {
    groups.push(digits.slice(index, index + 4).join(''));
  }
  return groups.join(' ');
}

/** The fingerprint of a pinned key, once computed; null for a key that is not 64 hex digits. */
export function useWorkspaceKeyFingerprint(keyHex: string | null | undefined): string | null {
  const [result, setResult] = useState<{ key: string; fingerprint: string | null } | null>(null);
  useEffect(() => {
    if (!keyHex) return;
    let live = true;
    workspaceKeyFingerprint(keyHex)
      .then((fingerprint) => {
        if (live) setResult({ key: keyHex, fingerprint });
      })
      .catch(() => {
        if (live) setResult({ key: keyHex, fingerprint: null });
      });
    return () => {
      live = false;
    };
  }, [keyHex]);
  return result && result.key === keyHex ? result.fingerprint : null;
}
