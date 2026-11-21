import { readConfig, removeConfig } from '../api';
import { userActionHeaders } from './userAction';

export const SECRET_MASK = '••••••••';

export async function removeProviderSecret(key: string): Promise<void> {
  const body = { key, is_secret: true };
  await removeConfig({ body, headers: await userActionHeaders(), throwOnError: true });
  const remaining = await readConfig({ body, throwOnError: true });
  if (remaining.data === undefined) throw new Error('Could not verify removal');
  if (remaining.data !== null) throw new Error('Key is still present');
}
