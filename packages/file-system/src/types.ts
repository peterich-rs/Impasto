export type DirectoryScope = "documents" | "applicationSupport" | "cache" | "temp";
export type FileKind = "file" | "directory";
export type FileSystemErrorCode = "outside-scope" | "not-found" | "already-exists" | "not-a-file" |
  "not-a-directory" | "permission-denied" | "cancelled" | "host-unavailable" | "io";

export interface AppDirectories {
  readonly documents: string;
  readonly applicationSupport: string;
  readonly cache: string;
  readonly temp: string;
  readonly downloads: string | null;
}
export interface FileStat {
  readonly kind: FileKind;
  readonly size: number;
  readonly modifiedAtMs: number | null;
  readonly createdAtMs: number | null;
  readonly readonly: boolean;
}
export interface DirEntry { readonly name: string; readonly path: string; readonly kind: FileKind; readonly size: number }
export interface WriteOptions { readonly scope?: DirectoryScope; readonly atomic?: boolean }
export interface MkdirOptions { readonly scope?: DirectoryScope; readonly recursive?: boolean }
export type DeleteOptions = MkdirOptions;
export interface TransferOptions { readonly fromScope?: DirectoryScope; readonly toScope?: DirectoryScope; readonly overwrite?: boolean }
export interface ReadStreamOptions { readonly scope?: DirectoryScope; readonly chunkSize?: number; readonly signal?: AbortSignal }
export interface OpenWriterOptions { readonly scope?: DirectoryScope; readonly append?: boolean; readonly atomic?: boolean }
export interface PickFilesOptions { readonly multiple?: boolean; readonly accept?: readonly string[] }
export interface PickedFile { readonly name: string; readonly relativePath: string; readonly size: number; readonly mimeType: string | null }
export interface PickedDirectory { readonly name: string; readonly path: string }
export interface SaveFileOptions {
  readonly suggestedName: string;
  readonly mimeType?: string;
  readonly allowedExtensions?: readonly string[];
  readonly bytes?: Uint8Array;
  readonly sourcePath?: string;
  readonly scope?: DirectoryScope;
}
export interface FileWriter {
  write(chunk: Uint8Array | string): Promise<void>;
  flush(): Promise<void>;
  commit(): Promise<void>;
  abort(): Promise<void>;
}
