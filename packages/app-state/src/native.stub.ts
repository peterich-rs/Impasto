import type * as native from "./native.js";

export function getCurrentState(): string {
  return "active";
}

export const addEventListener: typeof native.addEventListener = () => {};
export const removeEventListener: typeof native.removeEventListener = () => {};
