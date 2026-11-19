import { previewProviderSecret, readConfig, removeConfig } from '../api';
import { userActionHeaders } from './userAction';

export const SECRET_MASK = '••••••••';

export async function loadProviderSecretPreview(provider: string, key: string): Promise<string> {
  const response = await previewProviderSecret({
    body: { provider, key },
    headers: await userActionHeaders(),
    throwOnError: true,
  });
  if (typeof response.data !== 'string') throw new Error('Invalid key preview');
  const chars = Array.from(response.data);
  if (
    response.data !== SECRET_MASK &&
    (chars.length !== 16 || chars.slice(4, 12).join('') !== SECRET_MASK)
  ) {
    throw new Error('Invalid key preview');
  }
  return response.data;
}

export async function removeProviderSecret(key: string): Promise<void> {
  const body = { key, is_secret: true };
  await removeConfig({ body, headers: await userActionHeaders(), throwOnError: true });
  const remaining = await readConfig({ body, throwOnError: true });
  if (remaining.data === undefined) throw new Error('Could not verify removal');
  if (remaining.data !== null) throw new Error('Key is still present');
}
