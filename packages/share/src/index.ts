import { driver } from "./driver";
import type { ShareFileInput, ShareOptions, ShareResult } from "./types";
export type * from "./types";

export async function share(options: ShareOptions): Promise<ShareResult> {
  const { status, activity } = await driver().share({ ...options, files: options.files?.slice() });
  return activity == null ? { status } : { status, activity };
}
export function shareText(text: string, options?: Pick<ShareOptions, "title" | "subject" | "anchor">): Promise<ShareResult> {
  return share({ ...options, text });
}
export function shareFiles(files: readonly ShareFileInput[], options?: Omit<ShareOptions, "files">): Promise<ShareResult> {
  return share({ ...options, files });
}
