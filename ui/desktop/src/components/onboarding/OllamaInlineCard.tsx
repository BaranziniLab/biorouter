import { useEffect, useRef, useState } from 'react';
import { useConfig } from '../ConfigContext';
import { toastService } from '../../toasts';
import { Button } from '../ui/button';
import { Progress } from '../ui/progress';
import {
  checkOllamaStatus,
  getOllamaDownloadUrl,
  pollForOllama,
  hasModel,
  pullOllamaModel,
  getPreferredModel,
  type PullProgress,
} from '../../utils/ollamaDetection';
import OnboardingCardShell, { type OnboardingCardChrome } from './OnboardingCardShell';

interface OllamaInlineCardProps {
  onSuccess: () => void;
  /** See `OnboardingCardShell`. Defaults to the standalone card. */
  chrome?: OnboardingCardChrome;
}

type ModelStatus = 'checking' | 'available' | 'not-available' | 'downloading';

export default function OllamaInlineCard({ onSuccess, chrome = 'card' }: OllamaInlineCardProps) {
  const { upsert } = useConfig();
  const [isChecking, setIsChecking] = useState(true);
  const [ollamaDetected, setOllamaDetected] = useState(false);
  const [isPolling, setIsPolling] = useState(false);
  const [isConnecting, setIsConnecting] = useState(false);
  const [modelStatus, setModelStatus] = useState<ModelStatus>('checking');
  const [downloadProgress, setDownloadProgress] = useState<PullProgress | null>(null);
  const stopPollingRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    const checkInitial = async () => {
      const status = await checkOllamaStatus();
      setOllamaDetected(status.isRunning);
      if (status.isRunning) {
        const modelAvailable = await hasModel(getPreferredModel());
        setModelStatus(modelAvailable ? 'available' : 'not-available');
      }
      setIsChecking(false);
    };
    checkInitial();
    return () => {
      if (stopPollingRef.current) stopPollingRef.current();
    };
  }, []);

  const handleInstallClick = () => {
    setIsPolling(true);
    stopPollingRef.current = pollForOllama(async (status) => {
      setOllamaDetected(status.isRunning);
      setIsPolling(false);
      const modelAvailable = await hasModel(getPreferredModel());
      setModelStatus(modelAvailable ? 'available' : 'not-available');
      toastService.success({
        title: 'Ollama Detected!',
        msg: 'Ollama is now running. You can connect to it.',
      });
    }, 3000);
  };

  const handleDownloadModel = async () => {
    setModelStatus('downloading');
    setDownloadProgress({ status: 'Starting download...' });
    const success = await pullOllamaModel(getPreferredModel(), (progress) => {
      setDownloadProgress(progress);
    });
    if (success) {
      setModelStatus('available');
      toastService.success({
        title: 'Model Downloaded!',
        msg: `Downloaded ${getPreferredModel()}`,
      });
    } else {
      setModelStatus('not-available');
      toastService.error({
        title: 'Download failed',
        msg: `Failed to download ${getPreferredModel()}. Try again.`,
        traceback: '',
      });
    }
    setDownloadProgress(null);
  };

  const handleConnectOllama = async () => {
    setIsConnecting(true);
    try {
      await upsert('BIOROUTER_PROVIDER', 'ollama', false);
      await upsert('BIOROUTER_MODEL', getPreferredModel(), false);
      await upsert('OLLAMA_HOST', 'localhost', false);
      toastService.success({
        title: 'Success!',
        msg: `Connected to Ollama with ${getPreferredModel()} model.`,
      });
      onSuccess();
    } catch (error) {
      console.error('Failed to connect to Ollama:', error);
      toastService.error({
        title: 'Connection failed',
        msg: `Failed to connect to Ollama: ${error instanceof Error ? error.message : String(error)}`,
        traceback: error instanceof Error ? error.stack || '' : '',
      });
      setIsConnecting(false);
    }
  };

  const detectedPill = (
    <span className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
      <span className="w-1.5 h-1.5 rounded-full bg-background-success" />
      Ollama running · {getPreferredModel()}
      {modelStatus === 'available'
        ? ' ready'
        : modelStatus === 'not-available'
          ? ' not installed'
          : ''}
    </span>
  );

  const notDetectedPill = (
    <span className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
      <span className="w-1.5 h-1.5 rounded-full bg-background-warning" />
      Ollama not detected
    </span>
  );

  return (
    <OnboardingCardShell
      chrome={chrome}
      titleId="ollama-setup-title"
      category="local"
      label="Local · Run on your computer"
      title="Ollama"
      description="Run open-source models on your own machine. Free, private, offline."
    >
      {isChecking ? (
        <div className="flex items-center gap-2 text-xs text-text-muted">
          <div className="w-3 h-3 border-2 border-current border-t-transparent rounded-full animate-spin flex-shrink-0" />
          <span>Checking for Ollama…</span>
        </div>
      ) : ollamaDetected ? (
        <div className="space-y-3">
          <div>{detectedPill}</div>

          {modelStatus === 'checking' && (
            <div className="flex items-center gap-2 text-xs text-text-muted">
              <div className="w-3 h-3 border-2 border-current border-t-transparent rounded-full animate-spin flex-shrink-0" />
              <span>Checking for model…</span>
            </div>
          )}

          {modelStatus === 'downloading' && downloadProgress && (
            <div className="rounded-md border border-border-subtle bg-background-default p-3">
              <p className="text-xs text-text-default">Downloading {getPreferredModel()}…</p>
              <p className="text-[11px] text-text-muted mt-1">{downloadProgress.status}</p>
              {/* Ollama's pull stream reports `total`/`completed` only once the
                  manifest resolves — before that (and during the verify/extract
                  phases) there is no denominator. That is exactly the
                  indeterminate case, and it used to render as NOTHING: the card
                  showed a status line with no bar at all, so a pull that had not
                  yet reported a size looked stalled. */}
              {downloadProgress.total && downloadProgress.completed ? (
                <>
                  <Progress
                    className="mt-2"
                    label={`Downloading ${getPreferredModel()}`}
                    tone="success"
                    value={(downloadProgress.completed / downloadProgress.total) * 100}
                  />
                  <p className="text-[11px] text-text-muted mt-1">
                    {Math.round((downloadProgress.completed / downloadProgress.total) * 100)}%
                  </p>
                </>
              ) : (
                <Progress
                  className="mt-2"
                  label={`Downloading ${getPreferredModel()}`}
                  tone="success"
                  indeterminate
                />
              )}
            </div>
          )}

          <div className="flex flex-col items-stretch gap-2 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-4 sm:gap-y-3">
            {modelStatus === 'not-available' && (
              <Button
                onClick={handleDownloadModel}
                variant="outline"
                className="h-9 w-full px-4 sm:w-auto"
              >
                Download {getPreferredModel()} (~11GB)
              </Button>
            )}
            {modelStatus === 'available' && (
              <Button
                onClick={handleConnectOllama}
                disabled={isConnecting}
                className="h-9 w-full px-4 sm:w-auto"
              >
                {isConnecting ? 'Connecting…' : 'Connect to Ollama'}
              </Button>
            )}
          </div>
        </div>
      ) : (
        <div className="space-y-3">
          <div>{notDetectedPill}</div>
          <div className="flex flex-col items-stretch gap-2 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-4 sm:gap-y-3">
            {isPolling ? (
              <div className="flex items-center gap-2 text-xs text-text-muted">
                <div className="w-3 h-3 border-2 border-current border-t-transparent rounded-full animate-spin flex-shrink-0" />
                <span>Waiting for Ollama to start…</span>
              </div>
            ) : (
              <a
                href={getOllamaDownloadUrl()}
                target="_blank"
                rel="noopener noreferrer"
                onClick={handleInstallClick}
                className="inline-flex h-9 w-full items-center justify-center rounded-md bg-background-medium px-4 text-sm text-text-default transition-colors hover:bg-background-strong sm:w-auto"
              >
                Install Ollama
              </a>
            )}
          </div>
        </div>
      )}
    </OnboardingCardShell>
  );
}
