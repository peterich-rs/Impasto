import * as native from "@impasto/file-system/native";
import type { PickedDirectory, PickedFile, PickFilesOptions, SaveFileOptions } from "./types";

export interface Dialogs {
  pickFiles(options?: PickFilesOptions): Promise<PickedFile[]>;
  pickDirectory(): Promise<PickedDirectory | null>;
  saveFile(options: SaveFileOptions): Promise<boolean>;
}
const platform: Dialogs = {
  pickFiles: (options) => native.pickFiles(options && { ...options, accept: options.accept?.slice() }),
  pickDirectory: native.pickDirectory,
  saveFile: (options) => native.saveFile({
    ...options,
    allowedExtensions: options.allowedExtensions?.slice(),
  }),
};
let mock: Dialogs | undefined;

export const dialogs = (): Dialogs => mock ?? platform;
export function installMock(driver: Dialogs): () => void {
  if (native.isFileSystemHostAvailable() || mock) throw new Error("A file dialog mock requires an unbound test host");
  mock = driver;
  return () => { if (mock === driver) mock = undefined; };
}
