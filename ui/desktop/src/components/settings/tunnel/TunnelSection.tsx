import { useState, useEffect } from 'react';
import { Button } from '../../ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../ui/dialog';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../ui/card';
import { QRCodeSVG } from 'qrcode.react';
import {
  Loader2,
  Copy,
  Check,
  ChevronDown,
  ChevronUp,
  Info,
  ExternalLink,
  QrCode,
} from '../../icons/app-icons';
import { errorMessage } from '../../../utils/conversionUtils';
import { startTunnel, stopTunnel, getTunnelStatus } from '../../../api/sdk.gen';
import type { TunnelInfo } from '../../../api/types.gen';
import { useConfig } from '../../ConfigContext';
import { useTransientFlag } from '../../../hooks/useTransientFlag';

const STATUS_MESSAGES = {
  idle: 'Tunnel is not running',
  starting: 'Starting tunnel...',
  running: 'Tunnel is active',
  error: 'Tunnel encountered an error',
  disabled: 'Tunnel is disabled',
} as const;

const IOS_APP_STORE_URL = 'https://apps.apple.com/us/app/biorouter-ai/id6752889295';

export default function TunnelSection() {
  const { refreshConfig } = useConfig();
  const [tunnelInfo, setTunnelInfo] = useState<TunnelInfo>({
    state: 'idle',
    url: '',
    hostname: '',
    secret: '',
  });
  const [showQRModal, setShowQRModal] = useState(false);
  const [showAppStoreQRModal, setShowAppStoreQRModal] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copiedUrl, markUrlCopied] = useTransientFlag(2000);
  const [copiedSecret, markSecretCopied] = useTransientFlag(2000);
  const [showDetails, setShowDetails] = useState(false);

  const refreshConfigAfterTunnelWrite = async () => {
    try {
      await refreshConfig();
    } catch (err) {
      console.error('Failed to refresh config after updating the tunnel:', err);
    }
  };

  useEffect(() => {
    const loadTunnelInfo = async () => {
      try {
        const { data } = await getTunnelStatus();
        if (data) {
          setTunnelInfo(data);
        }
      } catch (err) {
        const errorMsg = errorMessage(err, 'Failed to load tunnel status');
        setError(errorMsg);
        setTunnelInfo({ state: 'error', url: '', hostname: '', secret: '' });
      }
    };

    loadTunnelInfo();
  }, []);

  const handleToggleTunnel = async () => {
    if (tunnelInfo.state === 'running') {
      try {
        await stopTunnel();
        await refreshConfigAfterTunnelWrite();
        setTunnelInfo({ state: 'idle', url: '', hostname: '', secret: '' });
        setShowQRModal(false);
      } catch (err) {
        setError(errorMessage(err, 'Failed to stop tunnel'));
        try {
          const { data } = await getTunnelStatus();
          if (data) {
            setTunnelInfo(data);
          }
        } catch (statusErr) {
          console.error('Failed to fetch tunnel status after stop error:', statusErr);
        }
      }
    } else {
      setError(null);
      setTunnelInfo({ state: 'starting', url: '', hostname: '', secret: '' });

      try {
        const { data } = await startTunnel();
        await refreshConfigAfterTunnelWrite();
        if (data) {
          setTunnelInfo(data);
          setShowQRModal(true);
        }
      } catch (err) {
        const errorMsg = errorMessage(err, 'Failed to start tunnel');
        setError(errorMsg);
        setTunnelInfo({ state: 'error', url: '', hostname: '', secret: '' });
      }
    }
  };

  const copyToClipboard = async (text: string, type: 'url' | 'secret') => {
    try {
      await navigator.clipboard.writeText(text);
      if (type === 'url') {
        markUrlCopied();
      } else {
        markSecretCopied();
      }
    } catch (err) {
      console.error('Failed to copy to clipboard:', err);
    }
  };

  const getQRCodeData = () => {
    if (tunnelInfo.state !== 'running') return '';

    const configJson = JSON.stringify({
      url: tunnelInfo.url,
      secret: tunnelInfo.secret,
    });
    const urlEncodedConfig = encodeURIComponent(configJson);
    return `biorouter://configure?data=${urlEncodedConfig}`;
  };

  if (tunnelInfo.state === 'disabled') {
    return null;
  }

  return (
    <>
      <Card className="rounded-element">
        <CardHeader className="pb-0">
          <CardTitle className="mb-1">Remote access</CardTitle>
          <CardDescription className="flex flex-col gap-2">
            <div className="flex items-start gap-2 p-2 bg-background-info/10 border border-border-info/40 rounded">
              <Info className="h-4 w-4 text-text-info flex-shrink-0 mt-0.5" />
              <div className="text-xs text-text-info">
                <strong>Preview feature:</strong> Reach Biorouter from a mobile device over an
                encrypted tunnel.{' '}
                <a
                  href={IOS_APP_STORE_URL}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex items-center gap-1 underline hover:no-underline"
                >
                  Get the iOS app
                  <ExternalLink className="h-3 w-3" />
                </a>
                {' or '}
                <button
                  onClick={() => setShowAppStoreQRModal(true)}
                  className="inline-flex items-center gap-1 underline hover:no-underline"
                >
                  scan QR code
                  <QrCode className="h-3 w-3" />
                </button>
              </div>
            </div>
          </CardDescription>
        </CardHeader>
        <CardContent className="pt-4 px-4 space-y-4">
          {error && (
            <div className="p-3 bg-background-danger/10 border border-border-danger/40 rounded text-sm text-text-danger">
              {error}
            </div>
          )}

          <div className="flex items-center justify-between">
            <div>
              <h3 className="text-text-default text-xs">Tunnel status</h3>
              <p className="text-xs text-text-muted max-w-md mt-[2px]">
                {STATUS_MESSAGES[tunnelInfo.state]}
              </p>
            </div>
            <div className="flex items-center gap-2">
              {tunnelInfo.state === 'starting' ? (
                <Button disabled variant="secondary" size="sm">
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Starting...
                </Button>
              ) : tunnelInfo.state === 'running' ? (
                <>
                  <Button onClick={() => setShowQRModal(true)} variant="default" size="sm">
                    Show QR code
                  </Button>
                  <Button onClick={handleToggleTunnel} variant="destructive" size="sm">
                    Stop tunnel
                  </Button>
                </>
              ) : (
                <Button onClick={handleToggleTunnel} variant="default" size="sm">
                  {tunnelInfo.state === 'error' ? 'Retry' : 'Start Tunnel'}
                </Button>
              )}
            </div>
          </div>

          {tunnelInfo.state === 'running' && (
            <div className="p-3 bg-background-success/10 border border-border-success/40 rounded">
              <p className="text-xs text-text-success">
                <strong>URL:</strong> {tunnelInfo.url}
              </p>
            </div>
          )}
        </CardContent>
      </Card>

      <Dialog open={showQRModal} onOpenChange={setShowQRModal}>
        <DialogContent className="sm:max-w-[500px]">
          <DialogHeader>
            <DialogTitle>Remote access connection</DialogTitle>
          </DialogHeader>

          {tunnelInfo.state === 'running' && (
            <div className="py-4 space-y-4">
              <div className="flex justify-center">
                <div className="p-4 bg-background-default rounded-element">
                  <QRCodeSVG value={getQRCodeData()} size={200} />
                </div>
              </div>

              <DialogDescription className="text-center text-sm text-text-muted">
                Scan this QR code with the Biorouter mobile app. Do not share this code with anyone
                else as it is for your personal access.
              </DialogDescription>

              <div className="border-t pt-4">
                <button
                  onClick={() => setShowDetails(!showDetails)}
                  className="flex items-center justify-between w-full text-sm font-medium hover:opacity-70 transition-opacity"
                >
                  <span>Connection details</span>
                  {showDetails ? (
                    <ChevronUp className="h-4 w-4" />
                  ) : (
                    <ChevronDown className="h-4 w-4" />
                  )}
                </button>

                {showDetails && (
                  <div className="mt-3 space-y-3">
                    <div>
                      <h3 className="text-xs font-medium mb-1 text-text-muted">Tunnel URL</h3>
                      <div className="flex items-center gap-2">
                        <code className="flex-1 p-2 bg-background-medium rounded text-xs break-all overflow-hidden">
                          {tunnelInfo.url}
                        </code>
                        <Button
                          size="sm"
                          variant="ghost"
                          className="flex-shrink-0"
                          onClick={() => tunnelInfo.url && copyToClipboard(tunnelInfo.url, 'url')}
                        >
                          {copiedUrl ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                        </Button>
                      </div>
                    </div>

                    <div>
                      <h3 className="text-xs font-medium mb-1 text-text-muted">Secret key</h3>
                      <div className="flex items-center gap-2">
                        <code className="flex-1 p-2 bg-background-medium rounded text-xs break-all overflow-hidden">
                          {tunnelInfo.secret}
                        </code>
                        <Button
                          size="sm"
                          variant="ghost"
                          className="flex-shrink-0"
                          onClick={() =>
                            tunnelInfo.secret && copyToClipboard(tunnelInfo.secret, 'secret')
                          }
                        >
                          {copiedSecret ? (
                            <Check className="h-4 w-4" />
                          ) : (
                            <Copy className="h-4 w-4" />
                          )}
                        </Button>
                      </div>
                    </div>
                  </div>
                )}
              </div>
            </div>
          )}

          <DialogFooter>
            <Button variant="outline" onClick={() => setShowQRModal(false)}>
              Close
            </Button>
            <Button variant="destructive" onClick={handleToggleTunnel}>
              Stop tunnel
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={showAppStoreQRModal} onOpenChange={setShowAppStoreQRModal}>
        <DialogContent className="sm:max-w-[400px]">
          <DialogHeader>
            <DialogTitle>Download Biorouter iOS App</DialogTitle>
          </DialogHeader>

          <div className="py-4 space-y-4">
            <div className="flex justify-center">
              <div className="p-4 bg-background-default rounded-element">
                <QRCodeSVG value={IOS_APP_STORE_URL} size={200} />
              </div>
            </div>

            <DialogDescription className="text-center text-sm text-text-muted">
              Scan this QR code with your iPhone camera to install the Biorouter mobile app from the
              App Store
            </DialogDescription>

            <div className="text-center">
              <a
                href={IOS_APP_STORE_URL}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-2 text-sm text-text-info hover:underline"
              >
                <ExternalLink className="h-4 w-4" />
                Open in App Store
              </a>
            </div>
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={() => setShowAppStoreQRModal(false)}>
              Close
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
