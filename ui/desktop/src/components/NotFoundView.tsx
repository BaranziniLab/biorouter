import { useLocation, useNavigate } from 'react-router-dom';
import { MainPanelLayout } from './Layout/MainPanelLayout';
import { PageHeader } from './Layout/PageHeader';
import { ReadableContent } from './Layout/ReadableContent';
import { EmptyState } from './ui/empty-state';
import { Button } from './ui/button';
import { CircleHelp, Home } from './icons/app-icons';

/**
 * The longest a mistyped address is worth echoing back. Past this the string
 * stops being a hint and starts being a wall of text in a `max-w-sm` paragraph,
 * and a pasted deep link can be thousands of characters long.
 */
const MAX_ECHOED_PATH = 72;

/** The address, trimmed to something a sentence can hold. Exported for the test. */
export function echoPath(pathname: string): string {
  const path = pathname.startsWith('/') ? pathname : `/${pathname}`;
  return path.length > MAX_ECHOED_PATH ? `${path.slice(0, MAX_ECHOED_PATH - 1)}…` : path;
}

/**
 * What an unknown address renders — and the reason it exists is that, until
 * now, it rendered NOTHING.
 *
 * `App.tsx`'s route table had no catch-all, so `#/zzz`, `#/scheduler` (the real
 * route is `#/schedules`) and the two routes PR #184 retired all produced a
 * white window: `document.body.innerText.length === 0`, no sidebar, no header,
 * no error boundary, and a console warning as the only trace. There was no way
 * back except reloading the app.
 *
 * ⚠ **It is mounted INSIDE the app shell**, as the last child of the `/` route,
 * so the sidebar and the titlebar are still there and Home is one click away
 * whether or not the button below is used. A catch-all at the top level would
 * render the message on the same bare canvas the blank page had, which fixes
 * the wrong half of the defect: the missing page was never the problem, the
 * missing *way out* was.
 *
 * The two retired routes do not land here — they redirect to Home, because an
 * old bookmark or `biorouter://` link to a feature that shipped is a different
 * thing from a typo, and it should arrive somewhere useful rather than be told
 * off. See the route table in `App.tsx`.
 */
export default function NotFoundView() {
  const location = useLocation();
  const navigate = useNavigate();

  return (
    <MainPanelLayout>
      <div className="flex min-w-0 flex-1 flex-col overflow-y-auto">
        <PageHeader
          title="Page not found"
          description="This address does not name a page in Biorouter. It may be a typo, or a link to something that has since moved."
        />
        <ReadableContent size="chat" className="px-6 py-4">
          <EmptyState
            icon={CircleHelp}
            title="There is nothing at this address"
            description={`Nothing answers to ${echoPath(location.pathname)}. Everything Biorouter can show you is in the sidebar.`}
            actions={
              <Button onClick={() => navigate('/')}>
                <Home />
                Go to Home
              </Button>
            }
          />
        </ReadableContent>
      </div>
    </MainPanelLayout>
  );
}
