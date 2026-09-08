import { useState, useEffect } from 'react';
import { Button } from '../../ui/button';
import { Check } from '../../icons/app-icons';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../ui/dialog';

const HelpText = () => (
  <div className="text-xs text-text-muted leading-relaxed p-3 rounded-element bg-background-muted border border-border-subtle">
    <span className="font-medium text-text-default">.biorouterhints</span> gives Biorouter
    additional context about your project. The{' '}
    <span className="font-medium text-text-default">Developer</span> extension must be enabled
    (Extensions page), and chats must be restarted for changes to take effect.
  </div>
);

const ErrorDisplay = ({ error }: { error: Error }) => (
  <div className="text-xs text-text-danger p-3 rounded-element bg-background-danger/10 border border-border-danger/40">
    Error reading .biorouterhints: {error.message}
  </div>
);

const FileInfo = ({ filePath, found }: { filePath: string; found: boolean }) => (
  <div className="flex items-center gap-1.5 text-xs mb-2">
    {found ? (
      <span className="flex items-center gap-1 text-text-success font-medium flex-shrink-0">
        <Check className="w-3 h-3" />
        Found
      </span>
    ) : (
      <span className="text-text-muted flex-shrink-0">New file</span>
    )}
    <span className="text-text-muted font-mono truncate">{filePath}</span>
  </div>
);

const getBioRouterHintsFile = async (filePath: string) => await window.electron.readFile(filePath);

interface BioRouterHintsModalProps {
  directory: string;
  setIsBioRouterHintsModalOpen: (isOpen: boolean) => void;
}

export const BioRouterHintsModal = ({
  directory,
  setIsBioRouterHintsModalOpen,
}: BioRouterHintsModalProps) => {
  const biorouterHintsFilePath = `${directory}/.biorouterhints`;
  const [biorouterHintsFile, setBioRouterHintsFile] = useState<string>('');
  const [biorouterHintsFileFound, setBioRouterHintsFileFound] = useState<boolean>(false);
  const [biorouterHintsFileReadError, setBioRouterHintsFileReadError] = useState<string>('');
  const [isSaving, setIsSaving] = useState(false);
  const [saveSuccess, setSaveSuccess] = useState(false);

  useEffect(() => {
    const fetchBioRouterHintsFile = async () => {
      try {
        const { file, error, found } = await getBioRouterHintsFile(biorouterHintsFilePath);
        setBioRouterHintsFile(file);
        setBioRouterHintsFileFound(found);
        setBioRouterHintsFileReadError(found && error ? error : '');
      } catch (error) {
        console.error('Error fetching .biorouterhints file:', error);
        setBioRouterHintsFileReadError('Failed to access .biorouterhints file');
      }
    };
    if (directory) fetchBioRouterHintsFile();
  }, [directory, biorouterHintsFilePath]);

  const writeFile = async () => {
    setIsSaving(true);
    setSaveSuccess(false);
    try {
      await window.electron.writeFile(biorouterHintsFilePath, biorouterHintsFile);
      setSaveSuccess(true);
      setBioRouterHintsFileFound(true);
      setTimeout(() => setSaveSuccess(false), 3000);
    } catch (error) {
      console.error('Error writing .biorouterhints file:', error);
      setBioRouterHintsFileReadError('Failed to save .biorouterhints file');
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <Dialog open={true} onOpenChange={(open) => setIsBioRouterHintsModalOpen(open)}>
      <DialogContent className="sm:max-w-2xl max-h-[85vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>Project hints</DialogTitle>
          <DialogDescription>
            Configure <code className="font-mono text-xs">.biorouterhints</code> to give Biorouter
            additional context about your project
          </DialogDescription>
        </DialogHeader>

        <div className="flex-1 overflow-y-auto space-y-3 py-2">
          <HelpText />

          {biorouterHintsFileReadError ? (
            <ErrorDisplay error={new Error(biorouterHintsFileReadError)} />
          ) : (
            <div>
              <FileInfo filePath={biorouterHintsFilePath} found={biorouterHintsFileFound} />
              <textarea
                value={biorouterHintsFile}
                className="w-full h-72 border border-border-subtle rounded-element p-3 text-sm font-mono resize-none bg-background-default text-text-default placeholder:text-text-muted focus:border-border-strong transition-colors duration-150"
                onChange={(event) => setBioRouterHintsFile(event.target.value)}
                placeholder="# Project context for Biorouter&#10;# e.g. language, frameworks, coding style, important files..."
              />
            </div>
          )}
        </div>

        <DialogFooter className="border-t border-border-subtle pt-4 mt-1">
          {saveSuccess && (
            <span className="text-text-success text-xs flex items-center gap-1 mr-auto font-medium">
              <Check className="w-3.5 h-3.5" />
              Saved
            </span>
          )}
          <Button variant="ghost" size="sm" onClick={() => setIsBioRouterHintsModalOpen(false)}>
            Close
          </Button>
          <Button size="sm" onClick={writeFile} disabled={isSaving}>
            {isSaving ? 'Saving…' : 'Save'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
