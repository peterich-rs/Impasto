/** Not registered by the runtime. Paths stay inside `documentDirectory`. */
export interface FileSystem {
  documentDirectory(): string;
  read(path: string): Promise<Uint8Array>;
  write(path: string, bytes: Uint8Array): Promise<void>;
  delete(path: string): Promise<void>;
  pick(): Promise<{ name: string } | null>;
}

export const name = "FileSystem";
