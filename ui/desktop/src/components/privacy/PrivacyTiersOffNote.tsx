import { useNavigate } from 'react-router-dom';
import { Button } from '../ui/button';
import { Note } from '../ui/note';
import { usePrivacyTiersEnabled, usePrivacyTiersRecord } from '../ConfigContext';
import { PRIVACY_TIERS_OFF_CONSEQUENCE, privacyTiersOffCopy } from './privacyTiersOffCopy';
import { RecordedIn } from './RecordedIn';

/**
 * The standing statement that privacy tiers are OFF, above every composer (H3,
 * 2026-09-10 security test drive).
 *
 * The drive wrote `{"enabled": false}` into the switch's record from a chat's
 * `developer__shell`; at the next launch every gate was off and the app's only
 * trace was a badge suffix and the strip inside Settings → Privacy. The file
 * channel is DR-17's accepted risk and stays open. The silence is what this
 * closes: the note says the switch is off, where it is recorded, and — when the
 * app recorded no deliberate change — that it was turned off outside the app.
 *
 * ⚠ **No dismiss control, deliberately.** Anything that could hide it would
 * need somewhere to remember the dismissal, and every such place is a file an
 * agent with a shell can write; the only way to make this go away is to turn
 * the tiers back on, which is the safe direction for anyone to take. The same
 * reasoning `PinnedModelNote` records: the condition is standing, so the
 * statement of it is too.
 *
 * ⚠ **Visibility is the switch's alone.** It shows whenever
 * {@link usePrivacyTiersEnabled} reads off; the record only chooses the words.
 * A daemon that sends no record costs the explanation, never the notice.
 *
 * Mounted above BOTH composers the app has — every chat's (`BaseChat`, in the
 * slot `PinnedModelNote` uses) and Home's (`Hub`), because Home is the route
 * the app launches on and an off switch takes effect at a launch. On the
 * composer's own rails — NOT above the chat header, whose 44px band must stay
 * level with the sidebar's and the artifact strip's (`--chrome-height`), and
 * not as a new app-wide strip, which the `h-screen` routes inside the shell
 * would overflow.
 */
export function PrivacyTiersOffNote({
  className,
}: {
  /** Layout only — `mx-*`, `mb-*`. */
  className?: string;
}) {
  const enabled = usePrivacyTiersEnabled();
  const record = usePrivacyTiersRecord();
  const navigate = useNavigate();
  if (enabled) return null;

  const copy = privacyTiersOffCopy(record);
  return (
    <Note
      tone={copy.tone}
      role="status"
      testId="privacy-tiers-off-note"
      className={className}
      action={
        <Button
          type="button"
          size="sm"
          variant="outline"
          // Privacy is a section of the App tab; SettingsView maps this section.
          onClick={() => navigate('/settings', { state: { section: 'privacy' } })}
        >
          Privacy settings
        </Button>
      }
    >
      <strong>{copy.headline}</strong> {copy.how && <>{copy.how} </>}
      {PRIVACY_TIERS_OFF_CONSEQUENCE}
      <RecordedIn path={copy.path} />
    </Note>
  );
}
