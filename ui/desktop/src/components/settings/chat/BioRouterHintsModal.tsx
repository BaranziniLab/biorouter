import { useState, useEffect, useRef } from 'react';
import { Button } from '../../ui/button';
import { Field } from '../../ui/field';
import { Note } from '../../ui/note';
import { Textarea } from '../../ui/textarea';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { ModalShell } from '../../ModalShell';
import { useTransientFlag, useTransientValue } from '../../../hooks/useTransientFlag';
import { copyToClipboard } from '../../../utils/clipboard';
import { hintsDialogCopy } from './copy';

const getBioRouterHintsFile = async (filePath: string) => await window.electron.readFile(filePath);

interface BioRouterHintsModalProps {
  directory: string;
  setIsBioRouterHintsModalOpen: (isOpen: boolean) => void;
}

/**
 * Settings > Chat > Project > Project hints > Edit…: the project's `.biorouterhints` file in one
 * field. `ModalShell` `lg` (spec §3.13); the requirement that makes it take effect is the
 * field's one helper line, not a boxed paragraph.
 */
export const BioRouterHintsModal = ({
  directory,
  setIsBioRouterHintsModalOpen,
}: BioRouterHintsModalProps) => {
  const biorouterHintsFilePath = `${directory}/.biorouterhints`;
  const [biorouterHintsFile, setBioRouterHintsFile] = useState<string>('');
  const [biorouterHintsFileFound, setBioRouterHintsFileFound] = useState<boolean>(false);
  const [biorouterHintsFileReadError, setBioRouterHintsFileReadError] = useState<string>('');
  const [isSaving, setIsSaving] = useState(false);
  const [saveSuccess, markSaved, clearSaved] = useTransientFlag(3000);
  const [copyResult, noteCopyResult, clearCopyResult] = useTransientValue<'copied' | 'failed'>(
    3000
  );
  // The footer's own box, where the clipboard's last-resort fallback may put its hidden field:
  // inside the dialog, because its focus trap pulls focus back out of anything on <body>.
  const footerStartRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const fetchBioRouterHintsFile = async () => {
      try {
        const { file, error, found } = await getBioRouterHintsFile(biorouterHintsFilePath);
        setBioRouterHintsFile(file);
        setBioRouterHintsFileFound(found);
        setBioRouterHintsFileReadError(found && error ? error : '');
      } catch (error) {
        console.error('Error fetching .biorouterhints file:', error);
        setBioRouterHintsFileReadError(hintsDialogCopy.accessFailed);
      }
    };
    if (directory) fetchBioRouterHintsFile();
  }, [directory, biorouterHintsFilePath]);

  const writeFile = async () => {
    setIsSaving(true);
    clearSaved();
    try {
      await window.electron.writeFile(biorouterHintsFilePath, biorouterHintsFile);
      markSaved();
      setBioRouterHintsFileFound(true);
    } catch (error) {
      console.error('Error writing .biorouterhints file:', error);
      setBioRouterHintsFileReadError(hintsDialogCopy.saveFailed);
    } finally {
      setIsSaving(false);
    }
  };

  const copyPath = async () => {
    clearCopyResult();
    const landed = await copyToClipboard(biorouterHintsFilePath, footerStartRef.current);
    noteCopyResult(landed ? 'copied' : 'failed');
  };

  return (
    <ModalShell
      open
      onOpenChange={(open) => setIsBioRouterHintsModalOpen(open)}
      size="lg"
      purpose="form"
      title={hintsDialogCopy.title}
      subtitle={biorouterHintsFileFound ? hintsDialogCopy.found : hintsDialogCopy.newFile}
      footer={
        <>
          {/* The path is behind an action, not on a line (principle 10): Copy path, with the
              whole path in its tooltip so the person can see which file before it is copied. */}
          <div ref={footerStartRef} className="mr-auto flex min-w-0 items-center gap-3">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button variant="ghost" size="sm" onClick={copyPath}>
                  {hintsDialogCopy.copyPath}
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                <span className="font-mono">{biorouterHintsFilePath}</span>
              </TooltipContent>
            </Tooltip>
            {saveSuccess ? (
              <span className="text-supporting text-text-success" role="status">
                {hintsDialogCopy.saved}
              </span>
            ) : copyResult === 'copied' ? (
              <span className="text-supporting text-text-success" role="status">
                {hintsDialogCopy.pathCopied}
              </span>
            ) : copyResult === 'failed' ? (
              <span className="text-supporting text-text-danger" role="alert">
                {hintsDialogCopy.copyFailed}
              </span>
            ) : null}
          </div>
          <Button variant="secondary" onClick={() => setIsBioRouterHintsModalOpen(false)}>
            {hintsDialogCopy.close}
          </Button>
          <Button onClick={writeFile} disabled={isSaving}>
            {isSaving ? hintsDialogCopy.saving : hintsDialogCopy.save}
          </Button>
        </>
      }
    >
      {biorouterHintsFileReadError ? (
        <Note tone="danger" role="alert">
          {hintsDialogCopy.readError(biorouterHintsFileReadError)}
        </Note>
      ) : (
        <Field
          id="project-hints-contents"
          label={hintsDialogCopy.field}
          helper={hintsDialogCopy.helper}
        >
          <Textarea
            id="project-hints-contents"
            rows={12}
            value={biorouterHintsFile}
            onChange={(event) => setBioRouterHintsFile(event.target.value)}
            placeholder={hintsDialogCopy.placeholder}
            className="resize-none"
          />
        </Field>
      )}
    </ModalShell>
  );
};
