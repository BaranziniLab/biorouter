export const FIXTURE_PASSWORD: string;
export const SENTINELS: string[];
export function chromeMicros(unixMs: number): bigint;

export type FixtureCookie = {
  host: string;
  name: string;
  value: string;
  path?: string;
  secure?: boolean;
  httpOnly?: boolean;
  sameSite?: number;
  expiresMs: number | null;
  topFrameSiteKey?: string;
  plain?: boolean;
  version?: 'v10' | 'v11';
  expect: string;
};

export type ChromeUserDataFixture = {
  root: string;
  platform: 'darwin' | 'linux';
  password: string;
  now: number;
  cookies: FixtureCookie[];
  personalCookies: FixtureCookie[];
  secrets: string[];
};

export function buildChromeUserData(
  root: string,
  options?: { platform?: 'darwin' | 'linux'; now?: number; password?: string }
): ChromeUserDataFixture;
