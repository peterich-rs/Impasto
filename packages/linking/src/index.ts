import * as scene from "./linking.ts";

export interface Linking {
  getInitialURL(): string | null;
  openURL(url: string): Promise<void>;
  addEventListener(type: "url", listener: (event: { url: string }) => void): void;
  removeEventListener(type: "url", listener: (event: { url: string }) => void): void;
}

export const Linking = scene as unknown as Linking;
