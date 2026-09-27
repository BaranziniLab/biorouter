import { Users } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { EmptyState } from '../../ui/empty-state';
import { useCrew } from '../state/CrewControllerContext';
import { welcomeCopy } from './copy';
import { SetupScreen } from './parts';

/**
 * First run: no saved workspace. One accent action — Join a workspace — and hosting as a quiet
 * link, because most people join a workspace someone else hosts. Privacy is stated where it is
 * chosen (the Join and Host dialogs), not here.
 */
export function Welcome() {
  const { openDialog } = useCrew();
  return (
    <SetupScreen>
      <EmptyState
        icon={Users}
        title={welcomeCopy.title}
        description={welcomeCopy.body}
        actions={
          <div className="flex flex-col items-center gap-2">
            <Button type="button" onClick={() => openDialog({ kind: 'join' })}>
              {welcomeCopy.join}
            </Button>
            <Button type="button" variant="link" onClick={() => openDialog({ kind: 'host' })}>
              {welcomeCopy.host}
            </Button>
          </div>
        }
      />
    </SetupScreen>
  );
}
