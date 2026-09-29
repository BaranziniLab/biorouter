import { Users } from '../icons/app-icons';
import { EmptyState } from '../ui/empty-state';
import { SetupScreen } from './onboarding/parts';

/**
 * What the `/crew` route shows in a browser opened with `biorouter serve` (CROSSCUT-5).
 *
 * `docs/deployment/serve-decisions.md` **SD-8**: a control that can never work on this surface says
 * so before it is touched. Crew can never work here, and that is provable rather than likely:
 *
 * - Every Crew route on the daemon asks for proof that a person acted (`require_person` in
 *   `routes/crew.rs` and `routes/crew_profile.rs`), listing workspaces included.
 * - A `serve` daemon holds no key to check that proof against (SD-7: it is spawned with stdin
 *   closed), so it answers every Crew request `crew_human_authority_unavailable`.
 * - And `userActionHeaders()` sends no proof from a browser surface anyway.
 *
 * So the route mounts nothing that talks to the daemon here. It used to mount the whole Crew
 * controller, which asked for the saved connections and failed, and every control after that
 * failed the same way, with the daemon's sentence telling the reader to start a desktop launcher
 * on a computer the browser may not even be on.
 *
 * ⚠ **These strings are for the person in the browser.** The daemon's refusal is written for
 * whatever reads an error body; nothing here is copied back into it.
 */
export const crewNeedsDesktopCopy = {
  title: 'Crew needs the Biorouter desktop app',
  body:
    'This page is open in a web browser through biorouter serve. Every Crew action needs proof ' +
    'that you, not an AI agent, asked for it, and a browser has no way to give that proof. ' +
    'Open Crew in the Biorouter desktop app on your own computer, or use the biorouter crew ' +
    'commands in a terminal there.',
} as const;

export function CrewNeedsDesktop() {
  return (
    <div className="h-full min-h-0 overflow-y-auto">
      <SetupScreen>
        <EmptyState
          icon={Users}
          title={crewNeedsDesktopCopy.title}
          description={crewNeedsDesktopCopy.body}
        />
      </SetupScreen>
    </div>
  );
}
