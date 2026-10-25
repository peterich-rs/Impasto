export interface Clipboard {
  getString(): Promise<string>;
  setString(text: string): Promise<void>;
}
