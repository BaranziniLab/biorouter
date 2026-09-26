import { app } from 'electron';
import fs from 'node:fs';
import path from 'node:path';

const requestedRoot = process.env.BIOROUTER_DEV_PROFILE_ROOT;
if (requestedRoot) {
  if (app.isPackaged) throw new Error('Development profiles are unavailable in installed builds.');
  if (!path.isAbsolute(requestedRoot))
    throw new Error('BIOROUTER_DEV_PROFILE_ROOT must be an absolute dedicated profile directory.');
  const root = path.resolve(requestedRoot);
  const name = process.env.BIOROUTER_DEV_PROFILE_NAME || path.basename(root);
  if (!/^[a-zA-Z0-9_-]{1,40}$/.test(name))
    throw new Error('Use a short alphanumeric development profile name.');
  if (
    root === app.getPath('home') ||
    root === app.getPath('userData') ||
    root === path.parse(root).root
  ) {
    throw new Error(
      'A development profile must not reuse your home or installed application profile.'
    );
  }
  const paths = {
    userData: 'electron',
    sessionData: 'session',
    temp: 'temp',
    logs: 'logs',
    home: 'home',
  } as const;
  fs.mkdirSync(root, { recursive: true, mode: 0o700 });
  if (fs.lstatSync(root).isSymbolicLink())
    throw new Error('Development profile root must not be a symbolic link.');
  for (const [key, directory] of Object.entries(paths)) {
    const location = path.join(root, directory);
    fs.mkdirSync(location, { recursive: true, mode: 0o700 });
    if (fs.lstatSync(location).isSymbolicLink())
      throw new Error(`Development profile ${key} must not be a symbolic link.`);
    app.setPath(key as keyof typeof paths, location);
  }
  app.setName(`Biorouter Dev · ${name}`);
  process.env.BIOROUTER_PATH_ROOT = path.join(root, 'biorouter');
  process.env.BIOROUTER_DISABLE_KEYRING = 'true';
  process.env.BIOROUTER_DEV_PROFILE_ROOT = root;
  process.env.BIOROUTER_DEV_PROFILE_NAME = name;
  delete process.env.BIOROUTER_EXTERNAL_BACKEND;
  delete process.env.BIOROUTER_EXTERNAL_BACKEND_URL;
  fs.writeFileSync(
    path.join(root, 'profile.json'),
    JSON.stringify(
      {
        name,
        root,
        pid: process.pid,
        userData: app.getPath('userData'),
        home: app.getPath('home'),
        biorouter: process.env.BIOROUTER_PATH_ROOT,
      },
      null,
      2
    ),
    { mode: 0o600 }
  );
}

export const developmentProfileRoot = requestedRoot ? path.resolve(requestedRoot) : undefined;
