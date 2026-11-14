/**
 * The inherited cloud dictation kept its provider choice under this localStorage key. That
 * feature sent audio to OpenAI or ElevenLabs, and PR-01 of
 * `docs/history/local-dictation-2026-11/implementation-plan.md`
 * deleted it (DD-89), so nothing reads the key any more and the renderer removes it at startup.
 *
 * Removing a key that is not there does nothing, so calling this on every start clears it once
 * without a marker of its own. The local dictation must never store anything under this name,
 * because this call would erase it on the next start.
 *
 * An `ELEVENLABS_API_KEY` a person saved stays in the credential store, unused; the release notes
 * say so and a person can delete it by hand.
 */
export const RETIRED_DICTATION_SETTINGS_KEY = 'dictation_settings';

export function clearRetiredDictationSettings(storage?: Storage): void {
  try {
    (storage ?? window.localStorage).removeItem(RETIRED_DICTATION_SETTINGS_KEY);
  } catch {
    // Storage can be blocked, in which case there is nothing to clear.
  }
}
