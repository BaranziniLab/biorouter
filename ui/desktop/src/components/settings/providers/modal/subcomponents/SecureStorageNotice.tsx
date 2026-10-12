import { Lock } from '../../../../icons/app-icons';

/**
 * Where a provider key is kept, in words that are true for every storage
 * backend the daemon can pick. The daemon uses the operating system's
 * credential store when one answers, and a plaintext `secrets.yaml` in its
 * config directory otherwise: every unpackaged build and dev profile, headless
 * Linux, SSH, WSL and `biorouter serve` (`config/base.rs`). The renderer is not
 * told which one is in use, so the copy must not promise the keychain or
 * encryption.
 */
export const KEY_STORAGE_NOTICE =
  "Keys are kept in your system keychain when one is available, otherwise in a private file in Biorouter's settings folder.";

/**
 * SecureStorageNotice - A reusable component that says where provider keys are kept.
 *
 * @param {Object} props - Component props
 * @param {string} [props.className] - Optional additional CSS classes
 * @param {string} [props.message] - Optional custom message (defaults to KEY_STORAGE_NOTICE)
 * @returns {JSX.Element} - The storage notice component
 */
export function SecureStorageNotice({ className = '', message = KEY_STORAGE_NOTICE }) {
  // Margins belong to the caller: a default `mt-*` here fought whatever the caller passed.
  return (
    <div className={`flex items-start gap-1.5 text-text-muted ${className}`}>
      <Lock className="mt-0.5 size-3.5 flex-shrink-0" aria-hidden />
      <span className="text-supporting">{message}</span>
    </div>
  );
}
