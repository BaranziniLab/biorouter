import { useLocation, useNavigate } from 'react-router-dom';
import { MainPanelLayout } from './Layout/MainPanelLayout';
import { PageHeader } from './Layout/PageHeader';
import { ReadableContent } from './Layout/ReadableContent';
import { EmptyState } from './ui/empty-state';
import { Button } from './ui/button';
import { CircleHelp } from './icons/app-icons';

/** The page's strings (copy rules, spec section 1). The empty-state title is read by
 * `App.routing.test.tsx`, so it changes only together with that test. */
const copy = {
  title: 'Page not found',
  info: 'This address does not name a page in Biorouter. It may be a typo, or a link to a page that moved.',
  emptyTitle: 'There is nothing at this address',
  emptyDescription: (path: string) => `Nothing answers to ${path}.`,
  goHome: 'Go to Home',
} as const;

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
    <MainPanelLayout removeTopPadding>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {/* The band is fixed, like every other view's; only the body scrolls. */}
        <PageHeader title={copy.title} info={copy.info} />
        <div className="min-h-0 flex-1 overflow-y-auto">
          <ReadableContent size="chat" className="px-6 pt-2 pb-6">
            <EmptyState
              icon={CircleHelp}
              title={copy.emptyTitle}
              description={copy.emptyDescription(echoPath(location.pathname))}
              actions={
                <Button variant="link" onClick={() => navigate('/')}>
                  {copy.goHome}
                </Button>
              }
            />
          </ReadableContent>
        </div>
      </div>
    </MainPanelLayout>
  );
}
