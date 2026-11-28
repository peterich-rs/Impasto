import * as native from "@impasto/file-system/native";
import { dialogs } from "./driver";
import type { DirectoryScope, FileWriter, OpenWriterOptions, PickFilesOptions, ReadStreamOptions, SaveFileOptions } from "./types";
export type * from "./types";

export const documentDirectory = native.documentDirectory;
export const directories = native.directories;
export const exists = native.exists;
export const stat = native.stat;
export const listDir = native.listDir;
export const mkdir = native.mkdir;
export const readText = native.readText;
export const write = native.write;
export const writeText = native.writeText;
export const copy = native.copy;
export const move = native.movePath;
const deletePath = native.delete;
export { deletePath as delete };

function bytes(value: Uint8Array | ArrayBuffer): Uint8Array {
  return value instanceof ArrayBuffer ? new Uint8Array(value) : value;
}
export async function read(path: string, scope?: DirectoryScope): Promise<Uint8Array> {
  return bytes(await native.read(path, scope));
}
export function append(path: string, value: Uint8Array | string, scope?: DirectoryScope): Promise<void> {
  return typeof value === "string" ? native.appendText(path, value, scope) : native.appendBytes(path, value, scope);
}
export function readStream(path: string, options?: ReadStreamOptions): ReadableStream<Uint8Array> {
  const reader = native.readStream(path, options && { scope: options.scope, chunkSize: options.chunkSize }, options?.signal).getReader();
  return new ReadableStream({
    async pull(controller) {
      try {
        const result = await reader.read();
        if (result.done) { reader.releaseLock(); controller.close(); }
        else controller.enqueue(bytes(result.value));
      } catch (error) { reader.releaseLock(); controller.error(error); }
    },
    async cancel(reason) { try { await reader.cancel(reason); } finally { reader.releaseLock(); } },
  });
}
export async function openWriter(path: string, options?: OpenWriterOptions): Promise<FileWriter> {
  const writer = await native.openWriter(path, options);
  return {
    write: (chunk) => typeof chunk === "string" ? writer.writeText(chunk) : writer.write(chunk),
    flush: () => writer.flush(),
    commit: () => writer.commit(),
    abort: () => writer.abort(),
  };
}
export const pickFiles = (options?: PickFilesOptions) => dialogs().pickFiles(options);
export const pickDirectory = () => dialogs().pickDirectory();
export const saveFile = (options: SaveFileOptions) => dialogs().saveFile(options);

export async function pick(): Promise<{ name: string } | null> {
  const [file] = await pickFiles();
  return file ? { name: file.relativePath } : null;
}

export const FileSystem = Object.freeze({
  documentDirectory, directories, exists, stat, listDir, mkdir, read, readText, write, writeText, append,
  copy, move, delete: deletePath, readStream, openWriter, pick, pickFiles, pickDirectory, saveFile,
});
export type FileSystem = typeof FileSystem;
