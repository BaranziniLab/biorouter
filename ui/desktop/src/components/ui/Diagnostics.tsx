import React, { useEffect, useState } from 'react';
import { AlertTriangle, Download, Github, Loader2 } from '../icons/app-icons';
import { Button } from './button';
import { toastError, toastSuccess } from '../../toasts';
import { diagnostics, getSession, systemInfo } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './dialog';

interface DiagnosticsModalProps {
  isOpen: boolean;
  onClose: () => void;
  sessionId: string;
  /**
   * The chat's ratcheted privacy classification, used only to decide whether to
   * add the extra warning below.
   *
   * ⚠ The tier, and NOT the bound provider, unlike the non-private-model
   * disclosure gate in `BaseChat` whose comment warns against this exact field.
   * The two ask different questions. That gate asks "where will this chat's
   * words be sent", which is a property of the model. This asks "does the file
   * I am about to hand the user hold private content", which is a property of
   * what the chat has already touched, and the ratcheted tier is the only thing
   * that answers it.
   *
   * Absent means "not known yet", which reads the same as public: a warning
   * that appears on every chat is one nobody reads by the third time.
   *
   * ⚠ This is a SEED, not the answer. It comes from the session `useChatStream`
   * loaded when the chat opened, and the classification is a ratchet that fires
   * DURING a turn — so on the chat where the warning matters most (a new chat
   * that just became private by talking to a private model) this prop is still
   * the pre-ratchet value and says "public". That is exactly how the warning
   * came to be missing on a live private chat while `Diagnostics.test.tsx`,
   * which passes the tier in directly, stayed green: the test proves the
   * component, and nothing proved the wiring. The read below is the answer.
   */
  privacyTier?: string;
}

export const DiagnosticsModal: React.FC<DiagnosticsModalProps> = ({
  isOpen,
  onClose,
  sessionId,
  privacyTier,
}) => {
  // Opening this dialog is itself a user gesture, so the proof-of-user read the
  // tier gate requires (issue #56 Task 58) is one we are entitled to make here.
  // Seeded from the prop so the warning is right on the first paint whenever the
  // caller already knew, and corrected the moment the daemon answers.
  const [liveTier, setLiveTier] = useState<string | undefined>(privacyTier);

  useEffect(() => {
    if (!isOpen || !sessionId) return;
    let cancelled = false;
    void (async () => {
      try {
        const response = await getSession({
          path: { session_id: sessionId },
          headers: await userActionHeaders(),
        });
        if (!cancelled && response.data?.privacy_tier) {
          setLiveTier(response.data.privacy_tier);
        }
      } catch (error) {
        // Leave the seed in place. A failed read must not manufacture a claim in
        // either direction: inventing "private" puts a false statement in front
        // of the user, and forcing "public" hides the warning this exists for.
        console.error('[Diagnostics] Failed to read the session privacy tier:', error);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isOpen, sessionId]);

  const isPrivateChat = liveTier === 'private';
  const [isDownloading, setIsDownloading] = useState(false);
  const [isFilingBug, setIsFilingBug] = useState(false);

  const handleDownload = async () => {
    setIsDownloading(true);

    try {
      const response = await diagnostics({
        headers: await userActionHeaders(),
        path: { session_id: sessionId },
        throwOnError: true,
      });

      const archive = await response.data.arrayBuffer();
      const result = await window.electron.saveDiagnosticsBundle(sessionId, archive);

      if (result.error) {
        throw new Error(result.error);
      }
      if (result.canceled) {
        return;
      }

      toastSuccess({
        title: 'Diagnostics saved',
        msg: result.filePath
          ? `The diagnostics bundle was saved to ${result.filePath}.`
          : 'The diagnostics bundle was saved.',
      });
      onClose();
    } catch (error) {
      toastError({
        title: 'Diagnostics error',
        msg:
          error instanceof Error
            ? error.message
            : typeof error === 'string' && error.trim()
              ? error
              : 'Failed to generate diagnostics.',
      });
    } finally {
      setIsDownloading(false);
    }
  };

  const handleFileGitHubIssue = async () => {
    setIsFilingBug(true);

    try {
      const response = await systemInfo({ throwOnError: true });
      const info = response.data;

      const providerModel =
        info.provider && info.model
          ? `${info.provider}: ${info.model}`
          : info.provider || info.model || '[e.g. Google: gemini-1.5-pro]';

      const extensions =
        info.enabled_extensions.length > 0
          ? info.enabled_extensions.join(', ')
          : '[e.g. Computer Controller, Figma]';

      // ⚠ Both links used to be dead: repo-relative documentation paths pasted
      // after a `github.com/<org>/<repo>/` prefix, which resolves to nothing.
      // They point at the published docs site, which is where the repository's
      // own `.github/ISSUE_TEMPLATE/bug_report.md` sends people.
      const body = `**Describe the bug**

💡 Before filing, check common issues:  
http://biorouter.ucsf.edu/docs

📦 To help us debug faster, attach your **diagnostics zip** if possible.  
👉 Capture it from this dialog's **Generate diagnostics** button.

A clear and concise description of what the bug is.

---

**To Reproduce**
Steps to reproduce the behavior:
1. Go to '...'
2. Click on '....'
3. Scroll down to '....'
4. See error

---

**Expected behavior**
A clear and concise description of what you expected to happen.

---

**Screenshots**
If applicable, add screenshots to help explain your problem.

---

**Please provide the following information**
- **OS & Arch:** ${info.os} ${info.os_version} ${info.architecture}
- **Interface:** UI
- **Version:** ${info.app_version}
- **Extensions enabled:** ${extensions}
- **Provider & Model:** ${providerModel}

---

**Additional context**
Add any other context about the problem here.
`;

      const params = new URLSearchParams({
        template: 'bug_report.md',
        body: body,
        labels: 'bug',
      });

      window.open(
        `https://github.com/BaranziniLab/biorouter/issues/new?${params.toString()}`,
        '_blank'
      );
      onClose();
    } catch {
      toastError({
        title: 'Error',
        msg: 'Failed to get system information',
      });
    } finally {
      setIsFilingBug(false);
    }
  };

  const isBusy = isDownloading || isFilingBug;

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && !isBusy && onClose()}>
      <DialogContent dismissible={!isBusy} className="sm:max-w-lg">
        <DialogHeader className="mb-0">
          <div className="flex items-start gap-3">
            <div className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-container border border-border-subtle bg-background-warning/40 text-text-warning">
              <AlertTriangle size={20} />
            </div>
            <div className="min-w-0 flex-1">
              <DialogTitle>Report a problem</DialogTitle>
              <DialogDescription className="mt-2">
                You can download a diagnostics zip file to share with the team, or file a bug
                directly on GitHub with your system details pre-filled. A diagnostics report
                contains the following:
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>
        <div className="space-y-3 text-body text-text-muted">
          <ul className="list-inside list-disc space-y-1">
            <li>Basic system info</li>
            <li>Your current chat messages</li>
            <li>Recent log files</li>
            <li>Configuration settings</li>
          </ul>
          <p>
            <strong className="text-text-default">Warning:</strong> If your chat contains sensitive
            information, do not share the diagnostics file publicly.
          </p>
          {isPrivateChat && (
            <p
              className="rounded-container border border-border-subtle bg-background-warning/40 px-3 py-2.5 text-text-default"
              data-testid="diagnostics-private-warning"
            >
              <strong>This chat is private.</strong> The diagnostics file includes its messages and
              its log files. Read the file before you send it to anyone, and take out anything that
              should not leave this machine.
            </p>
          )}
          <p>If you file a bug, consider attaching the diagnostics report to it.</p>
          <p data-testid="diagnostics-agent-hint">
            You can also just say{' '}
            <strong className="text-text-default">&ldquo;report a bug&rdquo;</strong> in the chat.
            Biorouter will work out what went wrong, write the report, remove paths and credentials
            from it, and show you the exact text before anything is published.
          </p>
        </div>
        {isDownloading && (
          <div
            className="flex items-center gap-3 rounded-container border border-border-subtle bg-background-muted px-3 py-2.5 text-body text-text-muted"
            role="status"
          >
            <Loader2 className="h-4 w-4 shrink-0 animate-spin" />
            Preparing diagnostics bundle...
          </div>
        )}
        <DialogFooter className="sm:flex-wrap">
          <Button onClick={onClose} variant="outline" size="sm" disabled={isBusy}>
            Cancel
          </Button>
          <Button onClick={handleDownload} variant="outline" size="sm" disabled={isBusy}>
            <Download size={16} className="mr-1" />
            {isDownloading ? 'Generating...' : 'Generate diagnostics'}
          </Button>
          <Button
            onClick={handleFileGitHubIssue}
            variant="outline"
            size="sm"
            disabled={isBusy}
            className="bg-background-accent text-text-on-accent hover:bg-background-accent/90"
          >
            <Github size={16} className="mr-1" />
            {isFilingBug ? 'Opening...' : 'File Bug on GitHub'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
