import { app } from 'electron';
import fs from 'fs';
import path from 'path';
import type { WindowCanvasMode } from './windowCanvas';

export interface EnvToggles {
  BIOROUTER_SERVER__MEMORY: boolean;
  BIOROUTER_SERVER__COMPUTER_CONTROLLER: boolean;
}

export interface ExternalBiorouterdConfig {
  enabled: boolean;
  url: string;
  secret: string;
  /**
   * Issue #56 DR-16: the user-action key of a backend the app did NOT start.
   * Optional, and absent means the app has no proof of user for that daemon, so
   * every request that would raise a chat's privacy capability fails closed.
   * That is the right default for a backend someone else launched — its
   * user-proof is whatever the person who launched it chose.
   */
  userActionKey?: string;
}

export interface Settings {
  envToggles: EnvToggles;
  showMenuBarIcon: boolean;
  showDockIcon: boolean;
  enableWakelock: boolean;
  spellcheckEnabled: boolean;
  externalBiorouterd?: ExternalBiorouterdConfig;
  /**
   * The theme the app last showed, as reported by a renderer over
   * `set-window-canvas`. A new chat window is created with that theme's canvas
   * as its native background, so a frame that arrives late never exposes a
   * colour the app does not paint (see utils/windowCanvas.ts). Absent on a first
   * launch, when the window follows the OS like the page does.
   */
  windowCanvasMode?: WindowCanvasMode;
}

function getSettingsFile(): string {
  return path.join(app.getPath('userData'), 'settings.json');
}

const defaultSettings: Settings = {
  envToggles: {
    BIOROUTER_SERVER__MEMORY: false,
    BIOROUTER_SERVER__COMPUTER_CONTROLLER: false,
  },
  showMenuBarIcon: true,
  showDockIcon: true,
  enableWakelock: false,
  spellcheckEnabled: true,
};

// In-memory write-through cache of the parsed settings. `loadSettings` is
// called ~20x per launch (several on the startup critical path); without this
// each call did a synchronous existsSync + readFileSync + JSON.parse on the
// main thread. `saveSettings` is the only writer through this module, so the
// cache stays authoritative. Callers still receive an isolated clone, so the
// previous "fresh object per call" semantics are preserved exactly.
let cachedSettings: Settings | null = null;

// Settings management
export function loadSettings(): Settings {
  if (cachedSettings === null) {
    try {
      if (fs.existsSync(getSettingsFile())) {
        const data = fs.readFileSync(getSettingsFile(), 'utf8');
        cachedSettings = JSON.parse(data) as Settings;
      } else {
        cachedSettings = defaultSettings;
      }
    } catch (error) {
      console.error('Error loading settings:', error);
      cachedSettings = defaultSettings;
    }
  }
  return structuredClone(cachedSettings);
}

export function saveSettings(settings: Settings): void {
  try {
    fs.writeFileSync(getSettingsFile(), JSON.stringify(settings, null, 2));
    cachedSettings = structuredClone(settings);
  } catch (error) {
    console.error('Error saving settings:', error);
  }
}

export function updateEnvironmentVariables(envToggles: EnvToggles): void {
  if (envToggles.BIOROUTER_SERVER__MEMORY) {
    process.env.BIOROUTER_SERVER__MEMORY = 'true';
  } else {
    delete process.env.BIOROUTER_SERVER__MEMORY;
  }

  if (envToggles.BIOROUTER_SERVER__COMPUTER_CONTROLLER) {
    process.env.BIOROUTER_SERVER__COMPUTER_CONTROLLER = 'true';
  } else {
    delete process.env.BIOROUTER_SERVER__COMPUTER_CONTROLLER;
  }
}
