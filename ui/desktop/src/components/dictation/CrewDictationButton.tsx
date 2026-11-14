import { useCallback, useId, useRef, useState } from 'react';
import { getDictionarySettings } from '../../api';
import { isBrowserSurface } from '../../utils/surface';
import { userActionHeaders } from '../../utils/userAction';
import { isContextMenuKey, openContextMenuFromKeyboard } from '../ui/keyboardContextMenu';
import { ModalShell } from '../ModalShell';
import { Button } from '../ui/button';
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '../ui/context-menu';
import { DICTATION_ACTIONS, dictationMessage } from './copy';
import { DictationButton } from './DictationButton';
import type { DictationTarget } from './dictationSession';
import type { DictationComposerControl } from './useDictationComposer';

interface CrewDictationButtonProps {
  control: DictationComposerControl;
  target: () => DictationTarget;
  onSettings: () => void;
}

export async function crewPrivateTermsAllowed(): Promise<boolean> {
  if (isBrowserSurface()) return false;
  try {
    const headers = await userActionHeaders();
    if (!headers['X-User-Action']) return false;
    const response = await getDictionarySettings({ headers });
    return response.data?.authority.add_private.allowed === true;
  } catch {
    return false;
  }
}

export function CrewDictationButton({ control, target, onSettings }: CrewDictationButtonProps) {
  const messageId = useId();
  const [privateAllowed, setPrivateAllowed] = useState(false);
  const [confirmPrivate, setConfirmPrivate] = useState(false);
  const allowanceRead = useRef(0);
  const confirmedMenuChoice = useRef(false);

  const refreshPrivateAllowance = useCallback(async () => {
    const read = ++allowanceRead.current;
    setPrivateAllowed(false);
    const allowed = await crewPrivateTermsAllowed();
    if (allowanceRead.current === read) setPrivateAllowed(allowed);
  }, []);

  return (
    <>
      <ContextMenu onOpenChange={(open) => open && void refreshPrivateAllowance()}>
        <ContextMenuTrigger asChild>
          <span
            className="br-dictation-slot"
            onPointerEnter={() => void refreshPrivateAllowance()}
            onFocusCapture={() => void refreshPrivateAllowance()}
            onKeyDownCapture={(event) => {
              if (isContextMenuKey(event)) {
                event.preventDefault();
                openContextMenuFromKeyboard(event.currentTarget);
              }
            }}
          >
            <DictationButton
              control={control}
              setView={() => onSettings()}
              idleTooltip={dictationMessage('MSG-CREW-TIP')}
            />
          </span>
        </ContextMenuTrigger>
        {privateAllowed && !control.holdsComposer && (
          <ContextMenuContent>
            <ContextMenuItem
              onSelect={() => {
                confirmedMenuChoice.current = true;
                setConfirmPrivate(true);
              }}
            >
              {DICTATION_ACTIONS.includePrivateTerms}
            </ContextMenuItem>
          </ContextMenuContent>
        )}
      </ContextMenu>
      <ModalShell
        open={confirmPrivate}
        onOpenChange={(open) => {
          setConfirmPrivate(open);
          if (!open) confirmedMenuChoice.current = false;
        }}
        size="sm"
        purpose="info"
        title={DICTATION_ACTIONS.includePrivateTerms}
        describedBy={messageId}
        footer={
          <>
            <Button
              type="button"
              variant="secondary"
              onClick={() => {
                setConfirmPrivate(false);
                confirmedMenuChoice.current = false;
                control.press();
              }}
            >
              {DICTATION_ACTIONS.publicTermsOnly}
            </Button>
            <Button
              type="button"
              onClick={() => {
                setConfirmPrivate(false);
                if (confirmedMenuChoice.current) {
                  confirmedMenuChoice.current = false;
                  control.pressWithTarget({
                    ...target(),
                    target_kind: 'crew',
                    include_private_terms: true,
                  });
                }
              }}
            >
              {DICTATION_ACTIONS.include}
            </Button>
          </>
        }
      >
        <p id={messageId} className="text-body text-text-muted">
          {dictationMessage('MSG-PRIVATE-TERMS')}
        </p>
      </ModalShell>
    </>
  );
}
