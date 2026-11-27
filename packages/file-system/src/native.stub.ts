import type { AppDirectories, DeleteOptions, DirEntry, DirectoryScope, FileStat, MkdirOptions, OpenWriterOptions, PickedDirectory, PickedFile, PickFilesOptions, ReadStreamOptions, SaveFileOptions, TransferOptions, WriteOptions } from "./types";

function unavailable(): never { throw Object.assign(new Error("FileSystem requires an application sandbox"), { code: "host-unavailable" }); }
export function isFileSystemHostAvailable() { return false; }
export function documentDirectory(): string { return unavailable(); }
export function directories(): AppDirectories { return unavailable(); }
export async function exists(_path: string, _scope?: DirectoryScope): Promise<boolean> { return unavailable(); }
export async function stat(_path: string, _scope?: DirectoryScope): Promise<FileStat> { return unavailable(); }
export async function listDir(_path?: string, _scope?: DirectoryScope): Promise<DirEntry[]> { return unavailable(); }
export async function mkdir(_path: string, _options?: MkdirOptions): Promise<void> { unavailable(); }
export async function read(_path: string, _scope?: DirectoryScope): Promise<Uint8Array> { return unavailable(); }
export async function readText(_path: string, _scope?: DirectoryScope): Promise<string> { return unavailable(); }
export async function write(_path: string, _data: Uint8Array, _options?: WriteOptions): Promise<void> { unavailable(); }
export async function writeText(_path: string, _data: string, _options?: WriteOptions): Promise<void> { unavailable(); }
export async function appendBytes(_path: string, _bytes: Uint8Array, _scope?: DirectoryScope): Promise<void> { unavailable(); }
export async function appendText(_path: string, _text: string, _scope?: DirectoryScope): Promise<void> { unavailable(); }
export async function copy(_from: string, _to: string, _options?: TransferOptions): Promise<void> { unavailable(); }
export async function movePath(_from: string, _to: string, _options?: TransferOptions): Promise<void> { unavailable(); }
async function deletePath(_path: string, _options?: DeleteOptions): Promise<void> { unavailable(); }
export { deletePath as delete };
export function readStream(_path: string, _options?: ReadStreamOptions, _signal?: AbortSignal): ReadableStream<Uint8Array> {
  return new ReadableStream({ start(controller) { try { unavailable(); } catch (error) { controller.error(error); } } });
}
interface Writer {
  write(bytes: Uint8Array): Promise<void>;
  writeText(text: string): Promise<void>;
  flush(): Promise<void>;
  commit(): Promise<void>;
  abort(): Promise<void>;
}
export async function openWriter(_path: string, _options?: OpenWriterOptions): Promise<Writer> { return unavailable(); }
export async function pickFiles(_options?: PickFilesOptions): Promise<PickedFile[]> { return []; }
export async function pickDirectory(): Promise<PickedDirectory | null> { return null; }
export async function saveFile(_options: SaveFileOptions): Promise<boolean> { return false; }
