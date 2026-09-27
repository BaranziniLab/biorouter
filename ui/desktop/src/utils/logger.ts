import log from 'electron-log';
import path from 'node:path';
import { appendFileSync } from 'node:fs';
import { format } from 'node:util';
import { app } from 'electron';

log.transports.file.resolvePathFn = () => {
  if (!app) return path.join(process.env.HOME ?? '/tmp', '.biorouter', 'logs', 'main.log');
  return path.join(app.getPath('userData'), 'logs', 'main.log');
};

log.transports.file.level = app?.isPackaged ? 'info' : 'debug';
log.transports.console.level = app?.isPackaged ? false : 'debug';

// electron-log's file transport defaults to `sync: true`, i.e. one blocking
// `fs.writeFileSync` per line on whichever thread logged it. In the main process
// that is the thread running the window, and the noisy paths are exactly the ones
// that run at startup — a single update check emits ~180 lines (#88). Buffered
// async writes keep the event loop free.
log.transports.file.sync = false;

// Queued writes may not drain during a native modal dialog or before a hard
// crash. Fatal startup errors use the synchronous helper below; ordinary logs
// remain asynchronous and logging failures must not crash the application.
log.errorHandler?.startCatching?.({ showDialog: false });

export function logStartupFailure(error: unknown): void {
  try {
    const message = format('[Main] Fatal error during startup:', error);
    appendFileSync(
      log.transports.file.getFile().path,
      `${new Date().toISOString()} [error] ${message}\n`,
      'utf8'
    );
  } catch {
    // A logging failure must not prevent the startup error dialog or shutdown.
  }
}

export default log;
