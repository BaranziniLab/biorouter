import { useEffect, useRef, useState } from 'react';
import { useConfig } from '../ConfigContext';
import { toastService } from '../../toasts';
import { errorMessage } from '../../utils/conversionUtils';
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
import './onboarding.css';
import { Spinner } from '../ui/spinner';
import { ollamaCopy } from './copy';
import { StatusPill } from './StatusPill';

interface OllamaInlineCardProps {
  onSuccess: () => void;
}

type ModelStatus = 'checking' | 'available' | 'not-available' | 'downloading';

export default function OllamaInlineCard({ onSuccess }: OllamaInlineCardProps) {
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
        title: ollamaCopy.detectedToast,
        msg: ollamaCopy.detectedToastDetail,
      });
    }, 3000);
  };

  const handleDownloadModel = async () => {
    setModelStatus('downloading');
    setDownloadProgress({ status: ollamaCopy.startingDownload });
    const success = await pullOllamaModel(getPreferredModel(), (progress) => {
      setDownloadProgress(progress);
    });
    if (success) {
      setModelStatus('available');
      toastService.success({
        title: ollamaCopy.downloadedToast,
        msg: ollamaCopy.downloadedToastDetail(getPreferredModel()),
      });
    } else {
      setModelStatus('not-available');
      toastService.error({
        title: ollamaCopy.downloadFailed,
        msg: ollamaCopy.downloadFailedDetail(getPreferredModel()),
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
        title: ollamaCopy.connectedToast,
        msg: ollamaCopy.connectedToastDetail(getPreferredModel()),
      });
      onSuccess();
    } catch (error) {
      console.error('Failed to connect to Ollama:', error);
      toastService.error({
        title: ollamaCopy.connectFailed,
        msg: ollamaCopy.connectFailedDetail(errorMessage(error)),
        traceback: error instanceof Error ? error.stack || '' : '',
      });
      setIsConnecting(false);
    }
  };

  const detectedPill = (
    <StatusPill tone="success">{ollamaCopy.running(getPreferredModel(), modelStatus)}</StatusPill>
  );

  const notDetectedPill = <StatusPill tone="warning">{ollamaCopy.notDetected}</StatusPill>;

  return (
    <div className="min-w-0">
      {isChecking ? (
        <div className="flex items-center gap-2 text-supporting text-text-muted">
          <Spinner size={14} />
          <span>{ollamaCopy.checking}</span>
        </div>
      ) : ollamaDetected ? (
        <div className="space-y-3">
          <div>{detectedPill}</div>

          {modelStatus === 'checking' && (
            <div className="flex items-center gap-2 text-supporting text-text-muted">
              <Spinner size={14} />
              <span>{ollamaCopy.checkingModel}</span>
            </div>
          )}

          {modelStatus === 'downloading' && downloadProgress && (
            <div className="br-first-run-box br-fill-surface p-3">
              <p className="text-supporting text-text-default">
                Downloading {getPreferredModel()}…
              </p>
              <p className="text-supporting text-text-muted mt-1">{downloadProgress.status}</p>
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
                  <p className="text-supporting text-text-muted mt-1">
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

          {/* One action on the 32px rung, never stretched; the model's name is in the status
              line above, so the button says what it does and how much it downloads (FR-S-8). */}
          <div className="flex flex-wrap items-center gap-2">
            {modelStatus === 'not-available' && (
              <Button onClick={handleDownloadModel} variant="secondary">
                {ollamaCopy.downloadModel}
              </Button>
            )}
            {modelStatus === 'available' && (
              <Button onClick={handleConnectOllama} disabled={isConnecting}>
                {isConnecting ? ollamaCopy.connecting : ollamaCopy.use}
              </Button>
            )}
          </div>
        </div>
      ) : (
        <div className="space-y-3">
          <div>{notDetectedPill}</div>
          <div className="flex flex-wrap items-center gap-2">
            {isPolling ? (
              <div className="flex items-center gap-2 text-supporting text-text-muted">
                <Spinner size={14} />
                <span>{ollamaCopy.waiting}</span>
              </div>
            ) : (
              <Button asChild variant="secondary">
                <a
                  href={getOllamaDownloadUrl()}
                  target="_blank"
                  rel="noopener noreferrer"
                  onClick={handleInstallClick}
                >
                  {ollamaCopy.install}
                </a>
              </Button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
