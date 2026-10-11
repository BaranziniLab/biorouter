import fs from 'node:fs/promises';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { isPathContained } from './pathContainment';

export async function writeFileWithinRoots(
  filePath: string,
  content: unknown,
  roots: string[]
): Promise<void> {
  if (!filePath || !path.isAbsolute(filePath) || typeof content !== 'string') {
    throw new Error('Invalid file write request');
  }
  const resolvedPath = path.resolve(filePath);
  if (!isPathContained(resolvedPath, roots)) {
    throw new Error('File write is outside allowed directories');
  }
  await fs.mkdir(path.dirname(resolvedPath), { recursive: true });
  if (!isPathContained(resolvedPath, roots)) {
    throw new Error('File write is outside allowed directories');
  }
  const tmpPath = `${resolvedPath}.${randomUUID()}.tmp`;
  try {
    await fs.writeFile(tmpPath, content, { encoding: 'utf8', flag: 'wx' });
    await fs.rename(tmpPath, resolvedPath);
  } finally {
    await fs.unlink(tmpPath).catch(() => {});
  }
}
