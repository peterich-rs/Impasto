import { installMock } from "./driver";
import type { PickedDirectory, PickedFile, PickFilesOptions, SaveFileOptions } from "./types";

export function mockFileSystem(initial: { files?: PickedFile[]; directory?: PickedDirectory | null; saved?: boolean } = {}) {
  let files = initial.files ?? [];
  let directory = initial.directory ?? null;
  let saved = initial.saved ?? false;
  const picks: (PickFilesOptions | undefined)[] = [];
  const saves: SaveFileOptions[] = [];
  const restore = installMock({
    async pickFiles(options) { picks.push(options); return files.slice(0, options?.multiple ? files.length : 1); },
    async pickDirectory() { return directory; },
    async saveFile(options) { saves.push(options); return saved; },
  });
  return { setFiles(value: PickedFile[]) { files = value; },
    setDirectory(value: PickedDirectory | null) { directory = value; },
    setSaved(value: boolean) { saved = value; }, picks: picks as readonly (PickFilesOptions | undefined)[],
    saves: saves as readonly SaveFileOptions[], restore };
}
