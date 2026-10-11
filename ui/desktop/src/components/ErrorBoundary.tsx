import React from 'react';
import { Button } from './ui/button';
import { AlertTriangle } from './icons/app-icons';
import { errorMessage } from '../utils/conversionUtils';

// Capture unhandled promise rejections
window.addEventListener('unhandledrejection', (event) => {
  window.electron?.logInfo?.(`[UNHANDLED REJECTION] ${event.reason}`);
});

// Capture global errors
window.addEventListener('error', (event) => {
  window.electron?.logInfo?.(
    `[GLOBAL ERROR] ${event.message} at ${event.filename}:${event.lineno}:${event.colno}`
  );
});

export function ErrorUI({ error }: { error: string }) {
  const handleReload = () => {
    window.electron.reloadApp();
  };

  return (
    <div className="fixed inset-0 w-full h-full flex flex-col items-center justify-center gap-6 bg-background">
      <div className="flex flex-col items-center gap-4 max-w-[600px] text-center px-6">
        <div className="w-16 h-16 rounded-container bg-background-danger/10 flex items-center justify-center mb-2">
          <AlertTriangle className="w-8 h-8 text-text-danger" />
        </div>

        <h1 className="text-title text-text-default">Honk!</h1>

        {window?.appConfig?.get('BIOROUTER_VERSION') !== undefined ? (
          <p className="text-body text-text-muted mb-2">
            An error occurred in Biorouter v{window?.appConfig?.get('BIOROUTER_VERSION') as string}.
          </p>
        ) : (
          <p className="text-body text-text-muted mb-2">An error occurred.</p>
        )}

        <pre className="w-full overflow-auto whitespace-pre-wrap rounded-element border border-transparent bg-background-well p-4 text-code text-text-danger">
          {error}
        </pre>

        <Button onClick={handleReload}>Reload</Button>
      </div>
    </div>
  );
}

export class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null; hasError: boolean }
> {
  constructor(props: { children: React.ReactNode }) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error) {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    // Send error to main process
    window.electron?.logInfo?.(`[ERROR] ${error.toString()}\n${errorInfo.componentStack}`);
  }

  render() {
    if (this.state.hasError) {
      return <ErrorUI error={errorMessage(this.state.error || 'Unknown error')} />;
    }
    return this.props.children;
  }
}
