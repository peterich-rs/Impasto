export interface Linking {
  getInitialURL(): string | null;
  openURL(url: string): Promise<void>;
  addEventListener(type: "url", listener: (event: { url: string }) => void): void;
}
