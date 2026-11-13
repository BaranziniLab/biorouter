import { Lock } from '../../../../icons/app-icons';
import { InfoTip } from '../../../../ui/info-tip';

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

/** The notice's one visible line: true on every backend, and on the host of a browser session. */
export const KEY_STORAGE_LINE = 'Keys are saved on the machine running Biorouter.';

/** The InfoTip's name: "About where keys are kept". */
const KEY_STORAGE_LABEL = 'where keys are kept';

/**
 * SecureStorageNotice: where provider keys are kept, under every key form. One short line beside
 * the lock, and the keychain-or-file detail in the InfoTip after it (owner, 2026-11-13: one line
 * up front, the detail on hover). The provider page's band InfoTip says the same detail.
 *
 * @param {Object} props - Component props
 * @param {string} [props.className] - Optional additional CSS classes
 * @param {string} [props.message] - The InfoTip's help (defaults to KEY_STORAGE_NOTICE)
 * @returns {JSX.Element} - The storage notice component
 */
export function SecureStorageNotice({ className = '', message = KEY_STORAGE_NOTICE }) {
  // Margins belong to the caller: a default `mt-*` here fought whatever the caller passed.
  return (
    <div className={`flex items-center gap-1.5 text-text-muted ${className}`}>
      <Lock className="size-3.5 flex-shrink-0" aria-hidden />
      <span className="text-supporting">{KEY_STORAGE_LINE}</span>
      <InfoTip label={KEY_STORAGE_LABEL} help={message} />
    </div>
  );
}
