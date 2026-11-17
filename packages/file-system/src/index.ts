import * as scene from "./files.ts";

export interface FileSystem {
  documentDirectory(): string;
  read(path: string): Promise<Uint8Array>;
  write(path: string, bytes: Uint8Array): Promise<void>;
  delete(path: string): Promise<void>;
  pick(): Promise<{ name: string } | null>;
  addEventListener(type: string, listener: (e: unknown) => void): void;
  removeEventListener(type: string, listener: (e: unknown) => void): void;
}

export const FileSystem = scene as unknown as FileSystem;
