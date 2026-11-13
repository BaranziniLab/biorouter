import fs from 'node:fs/promises';
import path from 'node:path';
import { writeFileWithinRoots } from './writeFileWithinRoots';

type SelectedFile = {
  filePath: string;
  parentPath: string;
  parentDevice: bigint;
  parentInode: bigint;
};

type SaveRequest = {
  token: symbol;
  selection?: SelectedFile;
};

export class NativeSaveFileGrants<Sender extends object> {
  private requests = new WeakMap<Sender, SaveRequest>();

  begin(sender: Sender): symbol {
    const token = Symbol();
    this.requests.set(sender, { token });
    return token;
  }

  revoke(sender: Sender): void {
    this.requests.delete(sender);
  }

  async select(sender: Sender, token: symbol, filePath?: string): Promise<boolean> {
    const request = this.requests.get(sender);
    if (!request || request.token !== token) return false;
    if (!filePath || !path.isAbsolute(filePath)) {
      this.requests.delete(sender);
      return false;
    }

    try {
      const resolvedPath = path.resolve(filePath);
      const parentPath = await fs.realpath(path.dirname(resolvedPath));
      const parent = await fs.stat(parentPath, { bigint: true });
      if (!parent.isDirectory()) throw new Error('Invalid save directory');
      if (this.requests.get(sender) !== request) return false;
      request.selection = {
        filePath: resolvedPath,
        parentPath,
        parentDevice: parent.dev,
        parentInode: parent.ino,
      };
      return true;
    } catch {
      if (this.requests.get(sender) === request) this.requests.delete(sender);
      return false;
    }
  }

  async write(sender: Sender, filePath: string, content: unknown): Promise<boolean> {
    const selection = this.requests.get(sender)?.selection;
    if (!selection) return false;
    // Consume before filesystem awaits so concurrent writes cannot reuse a choice.
    this.requests.delete(sender);
    if (!path.isAbsolute(filePath) || path.resolve(filePath) !== selection.filePath) return false;

    const parentPath = await fs.realpath(path.dirname(selection.filePath));
    const parent = await fs.stat(parentPath, { bigint: true });
    if (
      parentPath !== selection.parentPath ||
      !parent.isDirectory() ||
      parent.dev !== selection.parentDevice ||
      parent.ino !== selection.parentInode
    ) {
      throw new Error('The selected save directory changed');
    }

    const target = path.join(selection.parentPath, path.basename(selection.filePath));
    await writeFileWithinRoots(target, content, [selection.parentPath]);
    return true;
  }
}
