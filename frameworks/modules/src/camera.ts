/**
 * Not registered by the runtime.
 * `preview` stays in the spec until an external texture exists.
 */
export interface Camera {
  capture(): Promise<{ width: number; height: number } | null>;
  addEventListener(type: "preview", listener: (frame: { width: number; height: number }) => void): void;
}

export const name = "Camera";
