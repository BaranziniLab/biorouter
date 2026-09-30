import { app, type App } from 'electron';
import fs from 'node:fs';
import path from 'node:path';

type ProfileApplication = Pick<App, 'setPath' | 'commandLine'>;

function privateDirectory(location: string): void {
  const metadata = fs.lstatSync(location);
  if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
    throw new Error('--user-data-dir requires a directory without symbolic links.');
  }
  if (
    process.platform !== 'win32' &&
    (metadata.uid !== process.getuid!() || (metadata.mode & 0o7777) !== 0o700)
  ) {
    throw new Error('--user-data-dir requires directories owned by this user with mode 0700.');
  }
}

export function applyUserDataDirectory(
  application: ProfileApplication,
  argv: readonly string[]
): string | undefined {
  const values: string[] = [];
  for (let index = 0; index < argv.length; index++) {
    const argument = argv[index];
    if (argument.startsWith('--user-data-dir=')) values.push(argument.slice(16));
    else if (argument === '--user-data-dir') values.push(argv[++index] ?? '');
  }
  if (values.length === 0) return undefined;
  if (values.length !== 1 || !values[0] || !path.isAbsolute(values[0])) {
    throw new Error('--user-data-dir requires one absolute existing private directory.');
  }
  privateDirectory(values[0]);
  const root = fs.realpathSync(values[0]);
  if (root === path.parse(root).root) {
    throw new Error('--user-data-dir must not be a filesystem root.');
  }
  const locations = {
    userData: root,
    sessionData: path.join(root, 'session'),
    logs: path.join(root, 'logs'),
    temp: path.join(root, 'temp'),
    crashDumps: path.join(root, 'crash-dumps'),
    home: path.join(root, 'home'),
    appData: path.join(root, 'app-data'),
  } as const;
  for (const location of Object.values(locations)) {
    if (location !== root && !fs.existsSync(location)) fs.mkdirSync(location, { mode: 0o700 });
    privateDirectory(location);
  }
  for (const [key, location] of Object.entries(locations)) {
    application.setPath(key as keyof typeof locations, location);
  }
  application.commandLine.appendSwitch('user-data-dir', root);
  return root;
}

export const applicationUserDataDirectory = applyUserDataDirectory(app, process.argv);
