export interface AppState {
  currentState: string;
  addEventListener(type: "change", listener: (state: string) => void): void;
  removeEventListener(type: "change", listener: (state: string) => void): void;
}
