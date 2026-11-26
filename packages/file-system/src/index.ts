import * as native from "@impasto/file-system/native";

export interface FileSystem extends Omit<typeof native, "read"> {
  read(...args: Parameters<typeof native.read>): Promise<Uint8Array>;
}

export const FileSystem: FileSystem = {
  ...native,
  async read(path) {
    const bytes = await native.read(path);
    return bytes instanceof ArrayBuffer ? new Uint8Array(bytes) : bytes;
  },
};
