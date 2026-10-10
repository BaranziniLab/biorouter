import { useState, useEffect } from 'react';
import { Button } from '../../ui/button';
import { Field } from '../../ui/field';
import { Note } from '../../ui/note';
import { Textarea } from '../../ui/textarea';
import { ModalShell } from '../../ModalShell';
import { useTransientFlag } from '../../../hooks/useTransientFlag';
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

  return (
    <ModalShell
      open
      onOpenChange={(open) => setIsBioRouterHintsModalOpen(open)}
      size="lg"
      purpose="form"
      title={hintsDialogCopy.title}
      subtitle={
        <>
          {biorouterHintsFileFound ? hintsDialogCopy.found : hintsDialogCopy.newFile}{' '}
          <span className="font-mono">{biorouterHintsFilePath}</span>
        </>
      }
      footer={
        <>
          {saveSuccess && (
            <span className="mr-auto text-supporting text-text-success" role="status">
              {hintsDialogCopy.saved}
            </span>
          )}
          <Button variant="outline" onClick={() => setIsBioRouterHintsModalOpen(false)}>
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
