// Node stand-in for `@impasto/devtools/host`. The panel stays off.

const emptyPage = () => ({ entries: [] as unknown[] });

export function enabled(): boolean {
  return false;
}

export function sourceMap(): string | null {
  return null;
}

export function revision(): number {
  return 0;
}

export function logs(_since?: number) {
  return emptyPage();
}

export function errors(_since?: number) {
  return emptyPage();
}

export function network(_since?: number) {
  return emptyPage();
}

export function frames(_since?: number) {
  return emptyPage();
}

export function flags() {
  return {
    layoutScan: false,
    pointerTrace: false,
    suppressDispatch: false,
  };
}

export function setFlags(_flags?: Record<string, boolean>): void {}

export function clear(): void {}

export function platform() {
  return {
    os: "test",
    osVersion: "",
    scale: 1,
    viewport: { w: 0, h: 0, dpr: 1 },
    insets: { left: 0, top: 0, right: 0, bottom: 0 },
    padding: { left: 0, top: 0, right: 0, bottom: 0 },
    viewInsets: { left: 0, top: 0, right: 0, bottom: 0 },
    appState: "active",
    log: "info",
    devtools: false,
    dump: null,
  };
}

export function versions() {
  return { hostshell: "", enginePin: "", quickjsPin: "" };
}

export function logSource(): string[] {
  return ["script", "layout", "host", "net"];
}

export function addEventListener(
  _type: string,
  _listener: (e: unknown) => void,
): void {}

export function removeEventListener(
  _type: string,
  _listener: (e: unknown) => void,
): void {}
